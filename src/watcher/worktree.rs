use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebouncedEvent, Debouncer, RecommendedCache};

/// Information about a known worktree
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// Events emitted by the WorktreeMonitor
#[derive(Debug)]
pub enum WorktreeEvent {
    Added(WorktreeInfo),
    Removed {
        name: String,
        path: PathBuf,
    },
    /// A worktree's HEAD changed to a different branch.
    BranchChanged {
        worktree: String,
        branch: String,
    },
    /// The .git/worktrees/ directory structure changed (worktree added or removed on disk)
    StructureChanged,
}

/// Parse a branch name from a HEAD file's content.
///
/// Returns `Some("branch-name")` for symbolic refs (`ref: refs/heads/branch-name`),
/// or `None` for detached HEAD (raw commit hash) or empty content.
pub fn read_branch_from_head(content: &str) -> Option<String> {
    content
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.to_string())
}

/// Read a worktree's info from `.git/worktrees/<name>/`.
///
/// The `gitdir` file contains the path to `<worktree-path>/.git`.
/// `HEAD` contains the branch ref or a detached HEAD hash.
pub fn read_worktree_info(git_worktrees_dir: &Path, name: &str) -> Option<WorktreeInfo> {
    let wt_dir = git_worktrees_dir.join(name);
    if !wt_dir.is_dir() {
        return None;
    }

    let gitdir_content = std::fs::read_to_string(wt_dir.join("gitdir")).ok()?;
    let gitdir = gitdir_content.trim();
    let worktree_path = Path::new(gitdir).parent()?.to_path_buf();

    let head_content = std::fs::read_to_string(wt_dir.join("HEAD")).ok()?;
    let branch = head_content
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.to_string());

    Some(WorktreeInfo {
        name: name.to_string(),
        path: worktree_path,
        branch,
    })
}

/// List all worktrees in `.git/worktrees/`.
///
/// Returns an empty vec if the directory does not exist or cannot be read.
pub fn list_worktrees(git_worktrees_dir: &Path) -> Vec<WorktreeInfo> {
    let Ok(entries) = std::fs::read_dir(git_worktrees_dir) else {
        return vec![];
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            read_worktree_info(git_worktrees_dir, &name)
        })
        .collect()
}

/// Monitors .git/worktrees/ for structural changes and detects
/// branch switches by watching HEAD files.
pub struct WorktreeMonitor {
    _structure_debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    head_watchers: HashMap<String, Debouncer<RecommendedWatcher, RecommendedCache>>,
    known_branches: HashMap<String, String>,
    known_worktrees: HashMap<String, WorktreeInfo>,
    /// Name of the main worktree, used to exclude it when computing linked-worktree sets.
    main_worktree_name: Option<String>,
    current_target: Option<String>,
    common_git_dir: PathBuf,
    git_worktrees_dir: PathBuf,
    event_tx: Sender<WorktreeEvent>,
    debounce: Duration,
}

impl WorktreeMonitor {
    /// Create a new `WorktreeMonitor` for the given repo root.
    ///
    /// Returns a receiver for `WorktreeEvent`s and the monitor handle (must be kept alive).
    pub fn new(repo_path: &Path, debounce: Duration) -> Result<(Receiver<WorktreeEvent>, Self)> {
        // Resolve the actual .git directory (handles linked worktrees where .git is a file).
        // Then find the common git dir to locate the worktrees/ subdirectory.
        let common_git_dir =
            crate::git::resolve_common_git_dir(repo_path).unwrap_or_else(|| repo_path.join(".git"));
        let git_worktrees_dir = common_git_dir.join("worktrees");
        let (event_tx, event_rx) = bounded::<WorktreeEvent>(16);

        // Watch .git/worktrees/ for structural changes (new/removed worktrees)
        let structure_tx = event_tx.clone();
        let mut structure_debouncer = new_debouncer(
            debounce,
            None,
            move |result: std::result::Result<Vec<DebouncedEvent>, Vec<notify::Error>>| {
                if let Ok(_events) = result {
                    let _ = structure_tx.try_send(WorktreeEvent::StructureChanged);
                }
            },
        )
        .context("Failed to create worktree structure debouncer")?;

        if !git_worktrees_dir.exists() {
            std::fs::create_dir_all(&git_worktrees_dir).ok();
        }

        structure_debouncer
            .watch(&git_worktrees_dir, RecursiveMode::NonRecursive)
            .context("Failed to watch .git/worktrees/")?;

        let mut monitor = Self {
            _structure_debouncer: structure_debouncer,
            head_watchers: HashMap::new(),
            known_branches: HashMap::new(),
            known_worktrees: HashMap::new(),
            main_worktree_name: None,
            current_target: None,
            common_git_dir: common_git_dir.clone(),
            git_worktrees_dir,
            event_tx,
            debounce,
        };

        monitor.scan_and_reconcile();

        // Also register the main worktree so it's a peer of linked worktrees.
        // This lets the monitor detect branch changes on main and fire switch events.
        let main_info = {
            let main_path = common_git_dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| repo_path.to_path_buf());
            let main_name = main_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "main".to_string());
            // Read the main worktree's branch from <common-git-dir>/HEAD
            let branch = std::fs::read_to_string(common_git_dir.join("HEAD"))
                .ok()
                .and_then(|content| read_branch_from_head(&content));
            WorktreeInfo {
                name: main_name,
                path: main_path,
                branch,
            }
        };
        if !monitor.known_worktrees.contains_key(&main_info.name) {
            if let Some(ref branch) = main_info.branch {
                monitor
                    .known_branches
                    .insert(main_info.name.clone(), branch.clone());
            }
            monitor.start_head_watcher(&main_info);
            monitor.main_worktree_name = Some(main_info.name.clone());
            monitor
                .known_worktrees
                .insert(main_info.name.clone(), main_info);
        }

        Ok((event_rx, monitor))
    }

    /// Scan .git/worktrees/ and emit Added/Removed events for any changes.
    pub fn scan_and_reconcile(&mut self) {
        let current = list_worktrees(&self.git_worktrees_dir);
        let current_names: std::collections::HashSet<String> =
            current.iter().map(|w| w.name.clone()).collect();
        // Linked worktrees are every known worktree except the main one.
        // Deriving from known_worktrees keeps a single source of truth and prevents
        // the main worktree from generating a spurious Removed event.
        let known_names: std::collections::HashSet<String> = self
            .known_worktrees
            .keys()
            .filter(|name| Some(name.as_str()) != self.main_worktree_name.as_deref())
            .cloned()
            .collect();

        // Detect removed
        for name in known_names.difference(&current_names) {
            // Invariant: every entry in known_names is also in known_worktrees,
            // so this expect should never fire.
            let path = self
                .known_worktrees
                .get(name)
                .map(|info| info.path.clone())
                .expect("known_worktrees must contain every name in known_names");
            self.known_worktrees.remove(name);
            self.known_branches.remove(name);
            self.head_watchers.remove(name);
            let _ = self.event_tx.try_send(WorktreeEvent::Removed {
                name: name.clone(),
                path: path.clone(),
            });
            tracing::info!(worktree = %name, path = ?path, "Worktree removed");
        }

        // Detect added
        for wt in &current {
            if !self.known_worktrees.contains_key(&wt.name) {
                if let Some(ref branch) = wt.branch {
                    self.known_branches.insert(wt.name.clone(), branch.clone());
                }
                self.start_head_watcher(wt);
                self.known_worktrees.insert(wt.name.clone(), wt.clone());
                let _ = self.event_tx.try_send(WorktreeEvent::Added(wt.clone()));
                tracing::info!(worktree = %wt.name, path = ?wt.path, "Worktree added");
            }
        }
    }

    /// Get the name of the currently targeted worktree.
    pub fn current_target(&self) -> Option<&str> {
        self.current_target.as_deref()
    }

    /// Set the name of the currently targeted worktree.
    pub fn set_current_target(&mut self, name: Option<String>) {
        self.current_target = name;
    }

    /// Returns true if the given branch differs from the last-known branch for this worktree.
    pub fn is_branch_change(&self, worktree: &str, branch: &str) -> bool {
        match self.known_branches.get(worktree) {
            Some(known) => known != branch,
            None => true,
        }
    }

    /// Update the last-known branch for a worktree.
    pub fn record_branch(&mut self, worktree: &str, branch: &str) {
        self.known_branches
            .insert(worktree.to_string(), branch.to_string());
    }

    /// Look up a known worktree by name.
    pub fn worktree_info(&self, name: &str) -> Option<&WorktreeInfo> {
        self.known_worktrees.get(name)
    }

    /// Start a HEAD file watcher for a worktree.
    ///
    /// For linked worktrees the HEAD file is at `<common-git-dir>/worktrees/<name>/HEAD`.
    /// For the main worktree it is at `<common-git-dir>/HEAD`.
    fn start_head_watcher(&mut self, wt: &WorktreeInfo) {
        let name = wt.name.clone();

        // Determine the HEAD file path.
        // Linked worktrees have an entry in git_worktrees_dir; the main worktree does not.
        let head_file = {
            let linked_head = self.git_worktrees_dir.join(&name).join("HEAD");
            if linked_head.exists() {
                linked_head
            } else {
                // Main worktree — HEAD is directly in the common git dir
                self.common_git_dir.join("HEAD")
            }
        };

        let watch_dir = head_file
            .parent()
            .expect("HEAD file must have a parent directory")
            .to_path_buf();

        let tx = self.event_tx.clone();
        let watcher_name = name.clone();
        let log_name = name.clone();
        let head_file_clone = head_file.clone();

        let debouncer = new_debouncer(
            self.debounce,
            None,
            move |result: std::result::Result<Vec<DebouncedEvent>, Vec<notify::Error>>| match result
            {
                Ok(events) => {
                    // Only react to changes to the HEAD file itself
                    let head_changed = events.iter().any(|e| {
                        e.event
                            .paths
                            .iter()
                            .any(|p| p.ends_with("HEAD") || p == &head_file_clone)
                    });
                    if !head_changed {
                        return;
                    }
                    tracing::debug!(worktree = %log_name, "HEAD file changed");
                    if let Ok(content) = std::fs::read_to_string(&head_file_clone) {
                        if let Some(branch) = read_branch_from_head(&content) {
                            let _ = tx.try_send(WorktreeEvent::BranchChanged {
                                worktree: watcher_name.clone(),
                                branch,
                            });
                        }
                    }
                }
                Err(errors) => {
                    for e in &errors {
                        tracing::warn!(worktree = %log_name, error = %e, "HEAD watcher error");
                    }
                }
            },
        );

        match debouncer {
            Ok(mut d) => match d.watch(&watch_dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.head_watchers.insert(name, d);
                    tracing::debug!(worktree = %wt.name, head = ?head_file, "HEAD watcher started");
                }
                Err(e) => {
                    tracing::warn!(worktree = %wt.name, head = ?head_file, error = %e, "Failed to watch HEAD file");
                }
            },
            Err(e) => {
                tracing::warn!(worktree = %wt.name, error = %e, "Failed to start HEAD watcher");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Write the minimal `gitdir` and `HEAD` files that `read_worktree_info` expects.
    fn setup_fake_worktree(base: &Path, name: &str, worktree_path: &Path, branch: Option<&str>) {
        let wt_dir = base.join(name);
        fs::create_dir_all(&wt_dir).unwrap();
        let gitdir = worktree_path.join(".git");
        fs::write(wt_dir.join("gitdir"), gitdir.to_string_lossy().as_ref()).unwrap();
        let head_content = match branch {
            Some(b) => format!("ref: refs/heads/{b}"),
            None => "abc1234def5678".to_string(),
        };
        fs::write(wt_dir.join("HEAD"), head_content).unwrap();
    }

    #[test]
    fn test_read_worktree_info_with_branch() {
        let tmp = tempdir().unwrap();
        let worktree_path = tmp.path().join("my-worktree");
        fs::create_dir_all(&worktree_path).unwrap();

        setup_fake_worktree(tmp.path(), "feat-branch", &worktree_path, Some("feat/foo"));

        let info = read_worktree_info(tmp.path(), "feat-branch").unwrap();
        assert_eq!(info.name, "feat-branch");
        assert_eq!(info.path, worktree_path);
        assert_eq!(info.branch, Some("feat/foo".to_string()));
    }

    #[test]
    fn test_read_worktree_info_detached_head() {
        let tmp = tempdir().unwrap();
        let worktree_path = tmp.path().join("detached");
        fs::create_dir_all(&worktree_path).unwrap();

        setup_fake_worktree(tmp.path(), "detached-wt", &worktree_path, None);

        let info = read_worktree_info(tmp.path(), "detached-wt").unwrap();
        assert_eq!(info.name, "detached-wt");
        assert_eq!(info.branch, None);
    }

    #[test]
    fn test_read_worktree_info_nonexistent() {
        let tmp = tempdir().unwrap();
        let result = read_worktree_info(tmp.path(), "does-not-exist");
        assert!(result.is_none());
    }

    #[test]
    fn test_list_worktrees() {
        let tmp = tempdir().unwrap();
        let path_a = tmp.path().join("worktree-a");
        let path_b = tmp.path().join("worktree-b");
        fs::create_dir_all(&path_a).unwrap();
        fs::create_dir_all(&path_b).unwrap();

        setup_fake_worktree(tmp.path(), "wt-a", &path_a, Some("main"));
        setup_fake_worktree(tmp.path(), "wt-b", &path_b, Some("develop"));

        let mut worktrees = list_worktrees(tmp.path());
        worktrees.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].name, "wt-a");
        assert_eq!(worktrees[0].branch, Some("main".to_string()));
        assert_eq!(worktrees[1].name, "wt-b");
        assert_eq!(worktrees[1].branch, Some("develop".to_string()));
    }

    #[test]
    fn test_list_worktrees_empty_dir() {
        let tmp = tempdir().unwrap();
        let worktrees = list_worktrees(tmp.path());
        assert!(worktrees.is_empty());
    }

    #[test]
    fn test_list_worktrees_no_dir() {
        let tmp = tempdir().unwrap();
        let nonexistent = tmp.path().join("nope");
        let worktrees = list_worktrees(&nonexistent);
        assert!(worktrees.is_empty());
    }

    #[test]
    fn test_read_branch_from_head_symbolic_ref() {
        assert_eq!(
            read_branch_from_head("ref: refs/heads/main\n"),
            Some("main".to_string())
        );
        assert_eq!(
            read_branch_from_head("ref: refs/heads/drew/feature-branch\n"),
            Some("drew/feature-branch".to_string())
        );
    }

    #[test]
    fn test_read_branch_from_head_detached() {
        assert_eq!(
            read_branch_from_head("abc1234def5678901234567890abcdef12345678\n"),
            None
        );
    }

    #[test]
    fn test_read_branch_from_head_empty() {
        assert_eq!(read_branch_from_head(""), None);
    }

    #[test]
    fn test_scan_and_reconcile_removed_event_carries_path() {
        let tmp = tempdir().unwrap();
        // Fake main gitdir at <tmp>/.git. No commondir file, so
        // resolve_common_git_dir returns this as the common dir.
        let git_dir = tmp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let (rx, mut monitor) =
            WorktreeMonitor::new(tmp.path(), Duration::from_millis(10)).unwrap();

        // Drain any startup events so subsequent try_recv only observes reconcile output.
        while rx.try_recv().is_ok() {}

        // Register a linked worktree on disk under .git/worktrees/my-wt.
        let wt_path = tmp.path().join("my-wt");
        fs::create_dir_all(&wt_path).unwrap();
        let worktrees_dir = git_dir.join("worktrees");
        fs::create_dir_all(&worktrees_dir).unwrap();
        setup_fake_worktree(&worktrees_dir, "my-wt", &wt_path, Some("feature"));

        monitor.scan_and_reconcile();
        // Drain the Added event emitted for "my-wt".
        while rx.try_recv().is_ok() {}

        // Simulate removal of the worktree metadata.
        fs::remove_dir_all(worktrees_dir.join("my-wt")).unwrap();
        monitor.scan_and_reconcile();

        // Expect a Removed event with both name and path populated.
        let mut saw_removed_with_path = false;
        while let Ok(ev) = rx.try_recv() {
            if let WorktreeEvent::Removed { name, path } = ev {
                assert_eq!(name, "my-wt");
                assert_eq!(path, wt_path);
                saw_removed_with_path = true;
            }
        }
        assert!(saw_removed_with_path, "expected Removed event with path");
    }

    #[test]
    fn test_head_file_branch_change_detection() {
        let tmp = tempdir().unwrap();
        let worktree_path = tmp.path().join("my-wt");
        fs::create_dir_all(&worktree_path).unwrap();
        setup_fake_worktree(tmp.path(), "my-wt", &worktree_path, Some("main"));

        let head_path = tmp.path().join("my-wt").join("HEAD");
        let content = fs::read_to_string(&head_path).unwrap();
        assert_eq!(read_branch_from_head(&content), Some("main".to_string()));

        fs::write(&head_path, "ref: refs/heads/drew/new-feature\n").unwrap();
        let content = fs::read_to_string(&head_path).unwrap();
        assert_eq!(
            read_branch_from_head(&content),
            Some("drew/new-feature".to_string())
        );
    }
}
