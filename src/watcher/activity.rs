//! Worktree listing and branch resolution helpers.

use std::path::Path;
use std::time::SystemTime;

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

/// Read the branch name from `<common_git_dir>/HEAD`.
fn read_main_head(common_git_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(common_git_dir.join("HEAD")).ok()?;
    content
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_string())
}
