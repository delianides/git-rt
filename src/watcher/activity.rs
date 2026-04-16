//! Worktree listing and branch resolution helpers.

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};

use super::worktree::{list_worktrees, WorktreeInfo};

/// Compute the last-activity timestamp for a worktree by stat-ing its
/// HEAD and index files. Returns `None` if HEAD cannot be stat'd.
pub fn worktree_last_activity(worktree_path: &Path) -> Option<SystemTime> {
    let git_dir = crate::git::resolve_git_dir(worktree_path)?;
    let head_mtime = std::fs::metadata(git_dir.join("HEAD"))
        .ok()?
        .modified()
        .ok()?;
    let index_mtime = std::fs::metadata(git_dir.join("index"))
        .ok()
        .and_then(|m| m.modified().ok());
    Some(std::cmp::max(head_mtime, index_mtime.unwrap_or(head_mtime)))
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

/// Resolve a `--branch` argument: find the first worktree (main or linked)
/// that has `branch` checked out.
///
/// When multiple worktrees match, the main worktree wins because
/// `list_all_worktrees` returns it first.
pub fn resolve_branch_arg(repo_path: &Path, branch: &str) -> Result<WorktreeInfo> {
    list_all_worktrees(repo_path)
        .into_iter()
        .find(|wt| wt.branch.as_deref() == Some(branch))
        .with_context(|| format!("No worktree found for branch: {branch}"))
}

/// Read the branch name from `<common_git_dir>/HEAD`.
fn read_main_head(common_git_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(common_git_dir.join("HEAD")).ok()?;
    content
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// Write the minimal `.git/HEAD` for a fake main worktree.
    fn setup_fake_main(repo_path: &Path, branch: Option<&str>) {
        let git_dir = repo_path.join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        let head = match branch {
            Some(b) => format!("ref: refs/heads/{b}\n"),
            None => "abc1234def5678\n".to_string(),
        };
        fs::write(git_dir.join("HEAD"), head).unwrap();
    }

    /// Write the minimal `gitdir` and `HEAD` for a fake linked worktree under
    /// `<repo>/.git/worktrees/<name>/`. `worktree_path` is the on-disk location
    /// of the linked worktree.
    fn setup_fake_linked(repo_path: &Path, name: &str, worktree_path: &Path, branch: Option<&str>) {
        let wt_dir = repo_path.join(".git").join("worktrees").join(name);
        fs::create_dir_all(&wt_dir).unwrap();
        let gitdir = worktree_path.join(".git");
        fs::write(wt_dir.join("gitdir"), gitdir.to_string_lossy().as_ref()).unwrap();
        let head_content = match branch {
            Some(b) => format!("ref: refs/heads/{b}\n"),
            None => "abc1234def5678\n".to_string(),
        };
        fs::write(wt_dir.join("HEAD"), head_content).unwrap();
    }

    #[test]
    fn resolve_branch_arg_matches_main_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        setup_fake_main(&repo, Some("main"));

        let info = resolve_branch_arg(&repo, "main").unwrap();
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.name, "repo");
    }

    #[test]
    fn resolve_branch_arg_matches_linked_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        setup_fake_main(&repo, Some("main"));

        let linked_path = tmp.path().join("feat-wt");
        fs::create_dir_all(&linked_path).unwrap();
        setup_fake_linked(&repo, "feat-wt", &linked_path, Some("feature/x"));

        let info = resolve_branch_arg(&repo, "feature/x").unwrap();
        assert_eq!(info.name, "feat-wt");
        assert_eq!(info.branch.as_deref(), Some("feature/x"));
    }

    #[test]
    fn resolve_branch_arg_first_match_wins() {
        // Construct a duplicate-branch scenario (not reachable via real git, but
        // possible with detached/forged state). Document that main wins because
        // `list_all_worktrees` returns it first.
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        setup_fake_main(&repo, Some("shared"));

        let linked_path = tmp.path().join("dup-wt");
        fs::create_dir_all(&linked_path).unwrap();
        setup_fake_linked(&repo, "dup-wt", &linked_path, Some("shared"));

        let info = resolve_branch_arg(&repo, "shared").unwrap();
        assert_eq!(info.name, "repo", "main worktree should win (listed first)");
    }

    #[test]
    fn resolve_branch_arg_not_found() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        setup_fake_main(&repo, Some("main"));

        let result = resolve_branch_arg(&repo, "nope");
        assert!(result.is_err());
    }
}
