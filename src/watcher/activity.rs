//! Rank worktrees by recent activity for cold-start auto-switch.
//!
//! Activity is the max of three git-native signals per worktree: HEAD
//! commit's committer time, HEAD ref mtime, and index mtime. Main
//! worktree wins ties, then alphabetical.

use std::path::Path;
use std::time::{Duration, SystemTime};

use super::worktree::{list_worktrees, WorktreeInfo};

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
/// Returns them sorted newest-first. Ties prefer the main worktree,
/// then break alphabetically by `info.name`. Worktrees with no
/// detectable activity sort last (same tiebreaker order).
pub fn rank_by_activity(worktrees: &[WorktreeInfo]) -> Vec<WorktreeActivity> {
    // Resolve the main worktree's canonical path from any worktree in the set.
    // `main_worktree_path` canonicalizes for linked worktrees, so compare paths
    // canonicalized on both sides — callers may pass un-canonicalized paths
    // (e.g. `/tmp` vs `/private/tmp` on macOS).
    let main_path = worktrees
        .first()
        .map(|w| crate::git::main_worktree_path(&w.path))
        .and_then(|p| p.canonicalize().ok());

    let mut ranked: Vec<WorktreeActivity> = worktrees
        .iter()
        .map(|info| WorktreeActivity {
            info: info.clone(),
            last_activity: compute_activity(&info.path),
        })
        .collect();

    ranked.sort_by(|a, b| {
        let is_main = |w: &WorktreeActivity| {
            main_path
                .as_ref()
                .and_then(|m| w.info.path.canonicalize().ok().map(|c| &c == m))
                .unwrap_or(false)
        };
        // Primary: activity desc (None sorts last).
        let activity = match (a.last_activity, b.last_activity) {
            (Some(ta), Some(tb)) => tb.cmp(&ta),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        // Tiebreak: main first, then alphabetical.
        activity
            .then_with(|| match (is_main(a), is_main(b)) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.info.name.cmp(&b.info.name))
    });

    ranked
}

/// Compute the last activity timestamp for a worktree.
/// Takes the max of three git-native signals:
/// 1. HEAD commit's committer time (primary),
/// 2. mtime of the worktree's HEAD ref file (catches branch creation/checkout),
/// 3. mtime of the worktree's index file (catches staging).
fn compute_activity(worktree_path: &Path) -> Option<SystemTime> {
    [
        head_commit_time(worktree_path),
        head_ref_mtime(worktree_path),
        index_mtime(worktree_path),
    ]
    .into_iter()
    .flatten()
    .max()
}

/// Get the mtime of the worktree's `.git/index` file.
/// For linked worktrees, `.git` is a file pointing to the gitdir.
fn index_mtime(worktree_path: &Path) -> Option<SystemTime> {
    let git_dir = crate::git::resolve_git_dir(worktree_path)?;
    let index_path = git_dir.join("index");
    std::fs::metadata(&index_path).ok()?.modified().ok()
}

/// Get the committer timestamp of the worktree's HEAD commit via gix.
/// Returns `None` for unborn branches, missing repos, or corrupt commits.
fn head_commit_time(worktree_path: &Path) -> Option<SystemTime> {
    let repo = gix::open(worktree_path).ok()?;
    let commit = repo.head_commit().ok()?;
    let committer = commit.committer().ok()?;
    let time = committer.time().ok()?;
    let secs: u64 = time.seconds.try_into().ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// Get the mtime of the worktree's HEAD file.
/// Main worktree: `<common_git_dir>/HEAD`.
/// Linked worktree: `<common_git_dir>/worktrees/<name>/HEAD`.
fn head_ref_mtime(worktree_path: &Path) -> Option<SystemTime> {
    let git_dir = crate::git::resolve_git_dir(worktree_path)?;
    let head_path = git_dir.join("HEAD");
    std::fs::metadata(&head_path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, SystemTime};
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

    #[test]
    fn ranks_by_head_commit_time() {
        let tmp = tempdir().unwrap();
        let main = tmp.path().join("repo");
        init_repo(&main);
        commit_empty(&main, "root");

        // Linked A committed some time ago.
        let linked_a = add_linked_worktree(&main, "older", "branch-a");
        commit_empty(&linked_a, "older-commit");
        // Backdate the HEAD + index of A so only committer time is fresh-ish.
        let a_git = main.join(".git").join("worktrees").join("older");
        backdate(&a_git.join("HEAD"), 10);
        backdate(&a_git.join("index"), 10);

        std::thread::sleep(Duration::from_secs(1));

        // Linked B committed more recently.
        let linked_b = add_linked_worktree(&main, "newer", "branch-b");
        commit_empty(&linked_b, "newer-commit");

        let worktrees = vec![
            worktree_info("main", main),
            worktree_info("older", linked_a),
            worktree_info("newer", linked_b),
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(
            ranked[0].info.name, "newer",
            "newest commit should rank first"
        );
    }

    #[test]
    fn staging_boosts_worktree() {
        let tmp = tempdir().unwrap();
        let main = tmp.path().join("repo");
        init_repo(&main);
        commit_empty(&main, "root");

        let linked = add_linked_worktree(&main, "other", "other-branch");
        commit_empty(&linked, "other-commit");

        // Backdate everything so only a fresh stage in main will move it ahead.
        let main_git = main.join(".git");
        let other_git = main_git.join("worktrees").join("other");
        for p in [
            main_git.join("HEAD"),
            main_git.join("index"),
            other_git.join("HEAD"),
            other_git.join("index"),
        ] {
            if p.exists() {
                backdate(&p, 30);
            }
        }

        // Sleep so the fresh index mtime is demonstrably later than the
        // (non-backdated) commit committer times for both worktrees.
        std::thread::sleep(Duration::from_secs(2));

        // Stage a file in main — rewrites main's index mtime to now.
        fs::write(main.join("staged.txt"), "hi").unwrap();
        git(&main, &["add", "staged.txt"]);

        let worktrees = vec![worktree_info("main", main), worktree_info("other", linked)];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(ranked[0].info.name, "main", "fresh stage should win");
    }

    #[test]
    fn main_wins_tiebreak_when_activity_equal() {
        let tmp = tempdir().unwrap();
        let main = tmp.path().join("repo");
        init_repo(&main);
        commit_empty(&main, "root");
        let linked = add_linked_worktree(&main, "aaa-linked", "branch-a");

        // Clamp all activity signals on both worktrees to the exact same instant.
        let main_git = main.join(".git");
        let linked_git = main_git.join("worktrees").join("aaa-linked");
        let ts = FileTime::from_unix_time(1_700_000_000, 0);
        for p in [
            main_git.join("HEAD"),
            main_git.join("index"),
            linked_git.join("HEAD"),
            linked_git.join("index"),
        ] {
            if p.exists() {
                set_file_mtime(&p, ts).unwrap();
            }
        }

        let worktrees = vec![
            // Put linked first and give it an alphabetically-earlier name —
            // only the main-preference tiebreaker can put main ahead.
            worktree_info("aaa-linked", linked),
            worktree_info("zzz-main", main),
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(
            ranked[0].info.name, "zzz-main",
            "main should win ties even when alphabetically later"
        );
    }

    #[test]
    fn missing_activity_sorts_last() {
        let tmp = tempdir().unwrap();
        let main = tmp.path().join("repo");
        init_repo(&main);
        commit_empty(&main, "root");

        let worktrees = vec![
            worktree_info("ghost", tmp.path().join("nonexistent")),
            worktree_info("real", main),
        ];
        let ranked = rank_by_activity(&worktrees);
        assert_eq!(ranked[0].info.name, "real");
        assert_eq!(ranked[1].info.name, "ghost");
        assert!(ranked[1].last_activity.is_none());
    }
}
