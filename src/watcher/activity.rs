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
    use filetime::{set_file_mtime, FileTime};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::SystemTime;
    use tempfile::tempdir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command failed");
        assert!(
            out.status.success(),
            "git {:?} failed in {:?}: stdout={} stderr={}",
            args,
            dir,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }

    fn init_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "-q", "-b", "main"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["config", "user.name", "Test"]);
        // Ensure commits don't require signing in case the user's global
        // config has commit.gpgsign=true or similar.
        git(path, &["config", "commit.gpgsign", "false"]);
        git(path, &["config", "tag.gpgsign", "false"]);
    }

    fn commit_empty(path: &Path, msg: &str) {
        git(path, &["commit", "--allow-empty", "-q", "-m", msg]);
    }

    fn backdate(path: &Path, days_ago: u64) {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - days_ago * 24 * 60 * 60;
        set_file_mtime(path, FileTime::from_unix_time(secs as i64, 0)).unwrap();
    }

    fn add_linked_worktree(main: &Path, name: &str, branch: &str) -> PathBuf {
        let wt_path = main.join(".worktrees").join(name);
        git(
            main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                wt_path.to_str().unwrap(),
            ],
        );
        wt_path
    }

    fn worktree_info(name: &str, path: PathBuf) -> WorktreeInfo {
        WorktreeInfo {
            name: name.to_string(),
            path,
            branch: None,
        }
    }

    #[test]
    fn branch_creation_boosts_main_worktree() {
        // Reproduces the reported bug: linked worktree is 2 months old,
        // user creates a fresh branch in main (HEAD moves, no commit yet).
        // Main must rank first via head_ref_mtime.
        let tmp = tempdir().unwrap();
        let main = tmp.path().join("repo");
        init_repo(&main);
        commit_empty(&main, "root");

        // Create a linked worktree and backdate all its activity signals.
        let linked = add_linked_worktree(&main, "stale", "old-branch");
        let common_git = main.join(".git");
        let linked_gitdir = common_git.join("worktrees").join("stale");
        backdate(&linked_gitdir.join("HEAD"), 60);
        backdate(&linked_gitdir.join("index"), 60);
        // Also backdate main's signals to simulate "haven't worked here in a while".
        backdate(&common_git.join("HEAD"), 90);
        backdate(&common_git.join("index"), 90);

        // User creates a new branch in main: HEAD is rewritten to now, no commit.
        git(&main, &["checkout", "-q", "-b", "fresh-branch"]);
        // `checkout -b` also refreshes `.git/index` mtime on this platform.
        // Push it back so only HEAD reflects the branch creation — that isolates
        // the signal `head_ref_mtime` is meant to catch.
        backdate(&common_git.join("index"), 90);

        let worktrees = vec![
            worktree_info("main", main.clone()),
            worktree_info("stale", linked),
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(
            ranked[0].info.name, "main",
            "main should win after branch creation refreshed HEAD mtime"
        );
    }
}
