//! Worktree enumeration via `git worktree list --porcelain`.
//!
//! Used by the in-app switch dialog (`src/ui/switch_dialog.rs`) to present
//! every worktree of the current repo as a switch target. This module is
//! distinct from `src/watcher/worktree.rs`, which tracks `.git/worktrees/`
//! activity for auto-follow.

use std::path::PathBuf;

use thiserror::Error;

/// One row from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
}

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("malformed `git worktree list` output: {0}")]
    Parse(String),
    #[error("`git worktree list` failed: {0}")]
    Command(#[from] std::io::Error),
    #[error("`git worktree list` exited non-zero: {0}")]
    NonZero(String),
}

/// Parse `git worktree list --porcelain` output into a list of entries.
///
/// Format (from `git help worktree`):
/// ```text
/// worktree /path/to/dir
/// HEAD <40-hex-sha>
/// branch refs/heads/<name>     # absent if detached
/// [bare]
/// [detached]
/// [locked [<reason>]]
/// [prunable [<reason>]]
/// <blank line>
/// ```
pub fn parse_porcelain(input: &str) -> Result<Vec<WorktreeEntry>, WorktreeError> {
    let mut out = Vec::new();
    let mut current: Option<WorktreeEntry> = None;

    for line in input.lines() {
        if line.is_empty() {
            if let Some(entry) = current.take() {
                out.push(entry);
            }
            continue;
        }

        let (key, value) = match line.split_once(' ') {
            Some((k, v)) => (k, Some(v)),
            None => (line, None),
        };

        match key {
            "worktree" => {
                let path = value.ok_or_else(|| {
                    WorktreeError::Parse("`worktree` line missing path".into())
                })?;
                current = Some(WorktreeEntry {
                    path: PathBuf::from(path),
                    head: String::new(),
                    branch: None,
                    bare: false,
                    detached: false,
                    locked: None,
                    prunable: None,
                });
            }
            "HEAD" => {
                let entry = current.as_mut().ok_or_else(|| {
                    WorktreeError::Parse("`HEAD` before `worktree`".into())
                })?;
                let sha = value.ok_or_else(|| {
                    WorktreeError::Parse("`HEAD` line missing sha".into())
                })?;
                entry.head = sha.to_string();
            }
            "branch" => {
                let entry = current.as_mut().ok_or_else(|| {
                    WorktreeError::Parse("`branch` before `worktree`".into())
                })?;
                let raw = value.ok_or_else(|| {
                    WorktreeError::Parse("`branch` line missing ref".into())
                })?;
                let name = raw.strip_prefix("refs/heads/").unwrap_or(raw).to_string();
                entry.branch = Some(name);
            }
            "bare" => {
                if let Some(entry) = current.as_mut() {
                    entry.bare = true;
                }
            }
            "detached" => {
                if let Some(entry) = current.as_mut() {
                    entry.detached = true;
                }
            }
            "locked" => {
                if let Some(entry) = current.as_mut() {
                    entry.locked = Some(value.unwrap_or("").to_string());
                }
            }
            "prunable" => {
                if let Some(entry) = current.as_mut() {
                    entry.prunable = Some(value.unwrap_or("").to_string());
                }
            }
            _ => {} // unknown keys are ignored, per worktrunk's parser
        }
    }

    if let Some(entry) = current {
        out.push(entry);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_normal_worktree() {
        let input = "worktree /repos/foo\nHEAD abc123\nbranch refs/heads/main\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/repos/foo"));
        assert_eq!(entries[0].head, "abc123");
        assert_eq!(entries[0].branch, Some("main".to_string()));
        assert!(!entries[0].bare);
        assert!(!entries[0].detached);
        assert!(entries[0].locked.is_none());
        assert!(entries[0].prunable.is_none());
    }

    #[test]
    fn parse_multiple_worktrees() {
        let input = "\
worktree /repos/foo
HEAD aaa
branch refs/heads/main

worktree /repos/foo/.worktrees/feat
HEAD bbb
branch refs/heads/feat-x

";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].branch.as_deref(), Some("feat-x"));
    }

    #[test]
    fn parse_no_trailing_blank_line() {
        // Git output may end without a final blank line.
        let input = "worktree /a\nHEAD aaa\nbranch refs/heads/main";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_bare_worktree() {
        let input = "worktree /repos/foo.git\nHEAD aaa\nbare\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert!(entries[0].bare);
        assert!(!entries[0].detached);
        assert!(entries[0].branch.is_none());
    }

    #[test]
    fn parse_detached_worktree() {
        let input = "worktree /a\nHEAD aaa\ndetached\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert!(entries[0].detached);
        assert!(entries[0].branch.is_none());
    }

    #[test]
    fn parse_locked_with_reason() {
        let input = "worktree /a\nHEAD aaa\nbranch refs/heads/main\nlocked stash in progress\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(entries[0].locked.as_deref(), Some("stash in progress"));
    }

    #[test]
    fn parse_locked_without_reason() {
        let input = "worktree /a\nHEAD aaa\nbranch refs/heads/main\nlocked\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(entries[0].locked.as_deref(), Some(""));
    }

    #[test]
    fn parse_prunable_with_reason() {
        let input =
            "worktree /a\nHEAD aaa\nbranch refs/heads/main\nprunable gitdir file missing\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(
            entries[0].prunable.as_deref(),
            Some("gitdir file missing")
        );
    }

    #[test]
    fn parse_branch_without_refs_heads_prefix() {
        // Some edge cases (very old git) emit just the branch name.
        let input = "worktree /a\nHEAD aaa\nbranch main\n\n";
        let entries = parse_porcelain(input).unwrap();
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_empty_input() {
        let entries = parse_porcelain("").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_missing_path_errors() {
        let err = parse_porcelain("worktree\nHEAD aaa\n\n").unwrap_err();
        assert!(matches!(err, WorktreeError::Parse(_)));
    }

    #[test]
    fn parse_head_before_worktree_errors() {
        let err = parse_porcelain("HEAD aaa\n\n").unwrap_err();
        assert!(matches!(err, WorktreeError::Parse(_)));
    }
}
