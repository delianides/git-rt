//! Rank worktrees by recent activity for cold-start auto-switch.
//!
//! Uses a hybrid signal: primary is the git index mtime (fast stat call,
//! covers staging/commits/checkouts), fallback is a capped recursive file
//! walk of the worktree directory. The result is a `WorktreeActivity` with
//! an optional `last_activity` timestamp.

use std::path::Path;
use std::time::{Duration, SystemTime};

use super::worktree::{list_worktrees, WorktreeInfo};

/// Maximum depth of the recursive walk fallback.
const MAX_WALK_DEPTH: usize = 6;

/// Files modified more than this long ago are skipped during the walk.
const MAX_FILE_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60); // 7 days

/// Directories skipped during the walk.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".worktrees"];

/// A worktree with its computed activity timestamp.
#[derive(Debug, Clone)]
pub struct WorktreeActivity {
    pub info: WorktreeInfo,
    pub last_activity: Option<SystemTime>,
}

/// List all worktrees for the given repo path, including the main worktree
/// and any linked worktrees under `.git/worktrees/`.
///
/// The main worktree is always listed first.
pub fn list_all_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let mut result = Vec::new();

    // Main worktree: use repo_path as its path, basename as name, resolve
    // branch from HEAD.
    let common_git_dir =
        crate::git::resolve_common_git_dir(repo_path).unwrap_or_else(|| repo_path.join(".git"));
    let main_path = common_git_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| repo_path.to_path_buf());
    let main_name = main_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".to_string());
    let main_branch = read_main_head(&common_git_dir);
    result.push(WorktreeInfo {
        name: main_name,
        path: main_path,
        branch: main_branch,
    });

    // Linked worktrees
    let git_worktrees_dir = common_git_dir.join("worktrees");
    if git_worktrees_dir.is_dir() {
        result.extend(list_worktrees(&git_worktrees_dir));
    }

    result
}

/// Read the branch name from `<common_git_dir>/HEAD`.
fn read_main_head(common_git_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(common_git_dir.join("HEAD")).ok()?;
    content
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_string())
}

/// Rank a list of worktrees by most recent activity.
///
/// Returns them sorted newest-first. Ties break alphabetically by `info.name`.
/// Worktrees with no detectable activity sort last.
pub fn rank_by_activity(worktrees: &[WorktreeInfo]) -> Vec<WorktreeActivity> {
    let mut ranked: Vec<WorktreeActivity> = worktrees
        .iter()
        .map(|info| WorktreeActivity {
            info: info.clone(),
            last_activity: compute_activity(&info.path),
        })
        .collect();

    ranked.sort_by(|a, b| match (a.last_activity, b.last_activity) {
        (Some(ta), Some(tb)) => tb.cmp(&ta).then_with(|| a.info.name.cmp(&b.info.name)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.info.name.cmp(&b.info.name),
    });

    ranked
}

/// Compute the last activity timestamp for a worktree at the given path.
/// Uses index mtime as the primary signal, falls back to a capped walk.
fn compute_activity(worktree_path: &Path) -> Option<SystemTime> {
    let index_mtime = index_mtime(worktree_path);
    let walk_mtime = newest_file_mtime(worktree_path);

    match (index_mtime, walk_mtime) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Get the mtime of the worktree's `.git/index` file.
/// For linked worktrees, `.git` is a file pointing to the gitdir.
fn index_mtime(worktree_path: &Path) -> Option<SystemTime> {
    let git_dir = crate::git::resolve_git_dir(worktree_path)?;
    let index_path = git_dir.join("index");
    std::fs::metadata(&index_path).ok()?.modified().ok()
}

/// Recursively walk the worktree to find the newest modified file.
/// Respects depth, age, and directory skip list.
fn newest_file_mtime(root: &Path) -> Option<SystemTime> {
    let now = SystemTime::now();
    let cutoff = now - MAX_FILE_AGE;
    let mut newest: Option<SystemTime> = None;
    walk_dir(root, 0, cutoff, &mut newest);
    newest
}

fn walk_dir(dir: &Path, depth: usize, cutoff: SystemTime, newest: &mut Option<SystemTime>) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if SKIP_DIRS.iter().any(|skip| name_str == *skip) {
            continue;
        }
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            walk_dir(&path, depth + 1, cutoff, newest);
        } else if let Ok(mtime) = metadata.modified() {
            if mtime < cutoff {
                continue;
            }
            match newest {
                Some(current) if mtime <= *current => {}
                _ => *newest = Some(mtime),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_rank_single_worktree_with_no_activity() {
        let tmp = tempdir().unwrap();
        let worktrees = vec![WorktreeInfo {
            name: "only".to_string(),
            path: tmp.path().join("nonexistent"),
            branch: None,
        }];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].info.name, "only");
        assert!(ranked[0].last_activity.is_none());
    }

    #[test]
    fn test_rank_by_file_mtime_newer_first() {
        let tmp = tempdir().unwrap();
        let path_a = tmp.path().join("a");
        let path_b = tmp.path().join("b");
        fs::create_dir_all(&path_a).unwrap();
        fs::create_dir_all(&path_b).unwrap();
        fs::write(path_a.join("file.txt"), "older").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        fs::write(path_b.join("file.txt"), "newer").unwrap();

        let worktrees = vec![
            WorktreeInfo {
                name: "a".to_string(),
                path: path_a,
                branch: None,
            },
            WorktreeInfo {
                name: "b".to_string(),
                path: path_b,
                branch: None,
            },
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(ranked[0].info.name, "b", "newer worktree should rank first");
        assert_eq!(ranked[1].info.name, "a");
    }

    #[test]
    fn test_rank_none_activity_sorts_last() {
        let tmp = tempdir().unwrap();
        let path_a = tmp.path().join("a");
        fs::create_dir_all(&path_a).unwrap();
        fs::write(path_a.join("file.txt"), "content").unwrap();

        let worktrees = vec![
            WorktreeInfo {
                name: "ghost".to_string(),
                path: tmp.path().join("nonexistent"),
                branch: None,
            },
            WorktreeInfo {
                name: "real".to_string(),
                path: path_a,
                branch: None,
            },
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(ranked[0].info.name, "real");
        assert_eq!(ranked[1].info.name, "ghost");
        assert!(ranked[1].last_activity.is_none());
    }

    #[test]
    fn test_rank_alphabetical_tiebreaker_when_both_none() {
        let tmp = tempdir().unwrap();
        let worktrees = vec![
            WorktreeInfo {
                name: "zebra".to_string(),
                path: tmp.path().join("z"),
                branch: None,
            },
            WorktreeInfo {
                name: "alpha".to_string(),
                path: tmp.path().join("a"),
                branch: None,
            },
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(ranked[0].info.name, "alpha");
        assert_eq!(ranked[1].info.name, "zebra");
    }

    #[test]
    fn test_walk_respects_skip_dirs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // Create a file inside target/ that's older
        let target_dir = root.join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("new.bin"), "old-inside-target").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        // Write a root-level file (should be the newest non-skipped file)
        fs::write(root.join("src.rs"), "content").unwrap();

        // Also write a very new file in target/ to prove it's skipped
        std::thread::sleep(Duration::from_millis(20));
        fs::write(target_dir.join("newest.bin"), "newest-but-skipped").unwrap();

        let newest = newest_file_mtime(root).unwrap();
        let root_mtime = fs::metadata(root.join("src.rs"))
            .unwrap()
            .modified()
            .unwrap();
        let target_mtime = fs::metadata(target_dir.join("newest.bin"))
            .unwrap()
            .modified()
            .unwrap();
        // Newest should be the root file (target/ is skipped)
        assert!(
            newest >= root_mtime - Duration::from_millis(5)
                && newest <= root_mtime + Duration::from_millis(5),
            "newest should be the root src.rs file, not target/newest.bin (newest={:?}, root_mtime={:?}, target_mtime={:?})",
            newest,
            root_mtime,
            target_mtime
        );
    }
}
