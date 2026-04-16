//! Worktree data types and listing helpers.

use std::path::{Path, PathBuf};

/// Information about a known worktree
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
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
}
