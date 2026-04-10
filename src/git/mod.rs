use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitFailure {
    #[error("Not a git repository: {0}")]
    NotARepo(PathBuf),

    /// Git environment is in flux (e.g., worktree cleanup in progress,
    /// index.lock present, refs being rewritten). The caller should hold
    /// the last known state and try again on the next refresh.
    #[error("Git environment changed: {0}")]
    EnvChange(String),

    /// A real failure: corrupt repo, I/O error, unexpected gix error, etc.
    #[error("Git operation failed: {0}")]
    Failed(String),
}

impl GitFailure {
    /// Returns true if this failure indicates a transient env change
    /// (not a fatal error).
    pub fn is_env_change(&self) -> bool {
        matches!(self, GitFailure::EnvChange(_))
    }
}

/// Status of a file relative to the git index/HEAD
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Staged,
    Conflicted,
}

/// A single file entry from git status with diff stats
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Relative path from repo root
    pub path: String,
    pub status: FileStatus,
    /// Lines added (from numstat)
    pub insertions: usize,
    /// Lines deleted (from numstat)
    pub deletions: usize,
}

/// Parsed diff output for a single file
#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub hunks: Vec<DiffHunk>,
}

/// A single diff hunk
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A line within a diff hunk
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    HunkHeader,
}

/// Resolve the actual `.git` directory for a repository path.
/// In a normal repo, this is `repo_path/.git/`.
/// In a linked worktree, `.git` is a file containing `gitdir: <path>`,
/// so we read and resolve it.
pub fn resolve_git_dir(repo_path: &Path) -> Option<PathBuf> {
    let git_dir = repo_path.join(".git");
    if git_dir.is_dir() {
        Some(git_dir)
    } else if git_dir.is_file() {
        let content = std::fs::read_to_string(&git_dir).ok()?;
        let path = content.strip_prefix("gitdir: ")?.trim();
        let p = PathBuf::from(path);
        if p.is_relative() {
            Some(repo_path.join(p))
        } else {
            Some(p)
        }
    } else {
        None
    }
}

/// For a linked worktree's gitdir (e.g. `/repo/.git/worktrees/foo`),
/// resolve back to the main repo's `.git` directory.
pub fn resolve_common_git_dir(repo_path: &Path) -> Option<PathBuf> {
    let git_dir = resolve_git_dir(repo_path)?;
    // Check for commondir file (present in linked worktrees)
    let commondir = git_dir.join("commondir");
    if commondir.is_file() {
        let content = std::fs::read_to_string(&commondir).ok()?;
        let path = content.trim();
        let p = PathBuf::from(path);
        if p.is_relative() {
            Some(git_dir.join(p).canonicalize().ok()?)
        } else {
            Some(p)
        }
    } else {
        // Already in the main repo
        Some(git_dir)
    }
}

/// Git repository handle backed by gix (gitoxide).
pub struct GitRepo {
    repo: gix::Repository,
    repo_path: PathBuf,
}

impl GitRepo {
    pub fn new(path: &Path) -> Result<Self> {
        let repo = gix::open(path).map_err(|_e| GitFailure::NotARepo(path.to_path_buf()))?;

        // Resolve the canonical work dir path for downstream methods that still
        // use filesystem paths (e.g., diff_untracked which reads the file).
        let repo_path = repo
            .work_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf());

        Ok(Self { repo, repo_path })
    }

    /// Get the current branch name, or "HEAD" if detached
    pub fn branch_name(&self) -> Result<String, GitFailure> {
        match self.repo.head_name() {
            Ok(Some(name)) => Ok(name.shorten().to_string()),
            Ok(None) => Ok("HEAD".to_string()),
            Err(e) => Err(GitFailure::EnvChange(format!("branch_name: {e}"))),
        }
    }

    /// Compute the current status of all changed files with numstat.
    pub fn status(&self) -> Result<Vec<FileEntry>> {
        let mut entries = Vec::new();

        // Get porcelain status for file statuses
        let status_output = Command::new("git")
            .args(["status", "--porcelain=v1", "-uall"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git status")?;

        let status_str = String::from_utf8_lossy(&status_output.stdout);

        // Get numstat for insertion/deletion counts (unstaged worktree changes)
        let numstat_output = Command::new("git")
            .args(["diff", "--numstat"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git diff --numstat")?;

        // Get numstat for staged changes (index vs HEAD)
        let cached_numstat_output = Command::new("git")
            .args(["diff", "--cached", "--numstat"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git diff --cached --numstat")?;

        let numstat_str = String::from_utf8_lossy(&numstat_output.stdout);
        let cached_numstat_str = String::from_utf8_lossy(&cached_numstat_output.stdout);

        // Build a map of path -> (insertions, deletions), combining staged + unstaged
        let mut stats: std::collections::HashMap<String, (usize, usize)> =
            std::collections::HashMap::new();

        for line in numstat_str.lines().chain(cached_numstat_str.lines()) {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let ins = parts[0].parse::<usize>().unwrap_or(0);
                let del = parts[1].parse::<usize>().unwrap_or(0);
                let path = parts[2].to_string();
                let entry = stats.entry(path).or_insert((0, 0));
                entry.0 += ins;
                entry.1 += del;
            }
        }

        // Parse porcelain status
        for line in status_str.lines() {
            if line.len() < 3 {
                continue;
            }

            let index_status = line.as_bytes()[0] as char;
            let worktree_status = line.as_bytes()[1] as char;
            let path = line[3..].to_string();

            let status = match (index_status, worktree_status) {
                ('?', '?') => FileStatus::Untracked,
                ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => FileStatus::Conflicted,
                ('A', _) => FileStatus::Added,
                ('D', _) | (_, 'D') => FileStatus::Deleted,
                ('R', _) => FileStatus::Renamed,
                (_, 'M') | ('M', _) => FileStatus::Modified,
                _ => FileStatus::Modified,
            };

            let (insertions, deletions) = stats.get(&path).copied().unwrap_or((0, 0));

            entries.push(FileEntry {
                path,
                status,
                insertions,
                deletions,
            });
        }

        // Sort: staged first, then by path
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(entries)
    }

    /// Compute the unified diff for a single file
    pub fn diff_file(&self, path: &str) -> Result<FileDiff> {
        let output = Command::new("git")
            .args(["diff", "--", path])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git diff")?;

        let diff_str = String::from_utf8_lossy(&output.stdout);

        if diff_str.is_empty() {
            // Might be untracked — show the whole file as additions
            let file_path = self.repo_path.join(path);
            if file_path.exists() {
                return self.diff_untracked(path);
            }
            return Ok(FileDiff::default());
        }

        Ok(parse_unified_diff(&diff_str))
    }

    /// Get the repository name (basename of the main repo, even in a linked worktree).
    /// Uses the common git dir to find the parent repo path.
    pub fn repo_name(&self) -> String {
        if let Some(common_dir) = resolve_common_git_dir(&self.repo_path) {
            // common_dir is e.g. /path/to/repo/.git — parent is the repo root
            if let Some(repo_root) = common_dir.parent() {
                if let Some(name) = repo_root.file_name() {
                    return name.to_string_lossy().to_string();
                }
            }
        }
        // Fallback to basename of repo_path
        self.repo_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Get the worktree name (basename of the worktree's work directory).
    /// Handles linked worktrees where the work dir differs from the main repo.
    pub fn worktree_name(&self) -> String {
        self.repo
            .work_dir()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.repo_name())
    }

    /// Get HEAD short SHA and commit subject line
    pub fn head_info(&self) -> Result<(String, String), GitFailure> {
        let commit = match self.repo.head_commit() {
            Ok(c) => c,
            Err(e) => return Err(GitFailure::EnvChange(format!("head_info: {e}"))),
        };

        let sha = commit.id().shorten_or_id().to_string();

        let message = commit
            .message()
            .map(|m| m.summary().to_string())
            .unwrap_or_default();

        Ok((sha, message))
    }

    /// Count the number of stash entries
    pub fn stash_count(&self) -> Result<usize> {
        let output = Command::new("git")
            .args(["stash", "list"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git stash list")?;

        let text = String::from_utf8_lossy(&output.stdout);
        let count = text.lines().filter(|l| !l.is_empty()).count();
        Ok(count)
    }

    /// Get ahead/behind counts relative to upstream.
    /// Returns None if there is no upstream configured.
    pub fn ahead_behind(&self) -> Result<Option<(usize, usize)>> {
        let output = Command::new("git")
            .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git rev-list")?;

        if !output.status.success() {
            // No upstream configured
            return Ok(None);
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let parts: Vec<&str> = text.split('\t').collect();
        if parts.len() == 2 {
            let ahead = parts[0].parse::<usize>().unwrap_or(0);
            let behind = parts[1].parse::<usize>().unwrap_or(0);
            Ok(Some((ahead, behind)))
        } else {
            Ok(None)
        }
    }

    /// Check if the repo is in a special state (rebase, merge, cherry-pick, etc.)
    pub fn repo_state(&self) -> Option<String> {
        match self.repo.state() {
            Some(gix::state::InProgress::ApplyMailbox) => Some("APPLYING MAILBOX".to_string()),
            Some(gix::state::InProgress::ApplyMailboxRebase) => Some("REBASING".to_string()),
            Some(gix::state::InProgress::Bisect) => Some("BISECTING".to_string()),
            Some(gix::state::InProgress::CherryPick) => Some("CHERRY-PICKING".to_string()),
            Some(gix::state::InProgress::CherryPickSequence) => Some("CHERRY-PICKING".to_string()),
            Some(gix::state::InProgress::Merge) => Some("MERGING".to_string()),
            Some(gix::state::InProgress::Rebase) => Some("REBASING".to_string()),
            Some(gix::state::InProgress::RebaseInteractive) => Some("REBASING".to_string()),
            Some(gix::state::InProgress::Revert) => Some("REVERTING".to_string()),
            Some(gix::state::InProgress::RevertSequence) => Some("REVERTING".to_string()),
            None => None,
        }
    }

    /// Create a synthetic diff for untracked files (all lines as additions)
    fn diff_untracked(&self, path: &str) -> Result<FileDiff> {
        let file_path = self.repo_path.join(path);
        let content = std::fs::read_to_string(&file_path).unwrap_or_default();

        let lines: Vec<DiffLine> = content
            .lines()
            .map(|l| DiffLine {
                kind: DiffLineKind::Addition,
                content: l.to_string(),
            })
            .collect();

        let line_count = lines.len();

        Ok(FileDiff {
            hunks: vec![DiffHunk {
                header: format!("@@ -0,0 +1,{line_count} @@ (new file)"),
                lines,
            }],
        })
    }
}

/// Parse a unified diff string into structured hunks
fn parse_unified_diff(raw: &str) -> FileDiff {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;

    for line in raw.lines() {
        if line.starts_with("@@") {
            // Save previous hunk
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current_hunk {
            let diff_line = if let Some(content) = line.strip_prefix('+') {
                DiffLine {
                    kind: DiffLineKind::Addition,
                    content: content.to_string(),
                }
            } else if let Some(content) = line.strip_prefix('-') {
                DiffLine {
                    kind: DiffLineKind::Deletion,
                    content: content.to_string(),
                }
            } else if let Some(content) = line.strip_prefix(' ') {
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: content.to_string(),
                }
            } else {
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: line.to_string(),
                }
            };
            hunk.lines.push(diff_line);
        }
        // Skip diff header lines (---, +++, diff --git, index, etc.)
    }

    // Don't forget the last hunk
    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    FileDiff { hunks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unified_diff() {
        let raw = r#"diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,4 @@
 fn main() {
-    println!("Hello, world!");
+    println!("Hello, git-rt!");
 }
"#;
        let diff = parse_unified_diff(raw);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].lines.len(), 4);
        assert_eq!(diff.hunks[0].lines[1].kind, DiffLineKind::Deletion);
        assert_eq!(diff.hunks[0].lines[2].kind, DiffLineKind::Addition);
    }

    #[test]
    fn test_parse_empty_diff() {
        let diff = parse_unified_diff("");
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn test_parse_diff_only_headers() {
        let raw = "diff --git a/file b/file\nindex abc..def 100644\n--- a/file\n+++ b/file\n";
        let diff = parse_unified_diff(raw);
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn test_parse_multiple_hunks() {
        let raw = r#"diff --git a/file b/file
--- a/file
+++ b/file
@@ -1,3 +1,3 @@
 line1
-old2
+new2
 line3
@@ -10,3 +10,4 @@
 line10
+added
 line11
 line12
"#;
        let diff = parse_unified_diff(raw);
        assert_eq!(diff.hunks.len(), 2);
        assert_eq!(diff.hunks[0].header, "@@ -1,3 +1,3 @@");
        assert_eq!(diff.hunks[1].header, "@@ -10,3 +10,4 @@");

        // First hunk: context, deletion, addition, context
        assert_eq!(diff.hunks[0].lines.len(), 4);
        assert_eq!(diff.hunks[0].lines[0].kind, DiffLineKind::Context);
        assert_eq!(diff.hunks[0].lines[1].kind, DiffLineKind::Deletion);
        assert_eq!(diff.hunks[0].lines[2].kind, DiffLineKind::Addition);
        assert_eq!(diff.hunks[0].lines[3].kind, DiffLineKind::Context);

        // Second hunk: context, addition, context, context
        assert_eq!(diff.hunks[1].lines.len(), 4);
        assert_eq!(diff.hunks[1].lines[1].kind, DiffLineKind::Addition);
    }

    #[test]
    fn test_parse_diff_line_content_strips_prefix() {
        let raw = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-removed line\n+added line\n";
        let diff = parse_unified_diff(raw);
        assert_eq!(diff.hunks[0].lines[0].content, "removed line");
        assert_eq!(diff.hunks[0].lines[1].content, "added line");
    }

    #[test]
    fn test_file_diff_default_is_empty() {
        let diff = FileDiff::default();
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn test_file_status_variants() {
        // Ensure all variants are constructible and cloneable
        let statuses = vec![
            FileStatus::Modified,
            FileStatus::Added,
            FileStatus::Deleted,
            FileStatus::Renamed,
            FileStatus::Untracked,
            FileStatus::Staged,
            FileStatus::Conflicted,
        ];
        for s in &statuses {
            let cloned = s.clone();
            assert_eq!(s, &cloned);
        }
    }

    #[test]
    fn test_branch_name_returns_string() {
        // Use the project repo itself for testing
        let repo = GitRepo::new(std::path::Path::new("."));
        if let Ok(repo) = repo {
            let branch = repo.branch_name();
            assert!(branch.is_ok());
            assert!(!branch.unwrap().is_empty());
        }
    }

    #[test]
    fn test_repo_name() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let name = repo.repo_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_worktree_name() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let name = repo.worktree_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_head_info() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let (sha, message) = repo.head_info().unwrap();
        assert!(!sha.is_empty());
        assert!(sha.len() <= 12);
        assert!(!message.is_empty());
    }

    #[test]
    fn test_stash_count_returns_zero_or_more() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let count = repo.stash_count().unwrap();
        assert!(count < 10000);
    }

    #[test]
    fn test_ahead_behind_no_panic() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let _result = repo.ahead_behind();
    }

    #[test]
    fn test_repo_state_clean() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let state = repo.repo_state();
        assert!(state.is_none() || !state.unwrap().is_empty());
    }

    #[test]
    fn test_resolve_git_dir_normal_repo() {
        // The current repo (or worktree) should resolve
        let result = resolve_git_dir(std::path::Path::new("."));
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_git_dir_nonexistent() {
        let result = resolve_git_dir(std::path::Path::new("/tmp/nonexistent-repo-xyz"));
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_common_git_dir() {
        // Should resolve to a valid git dir
        let result = resolve_common_git_dir(std::path::Path::new("."));
        assert!(result.is_some());
        // The common dir should contain a HEAD file
        assert!(result.unwrap().join("HEAD").exists());
    }

    #[test]
    fn test_gitfailure_is_env_change() {
        assert!(GitFailure::EnvChange("x".into()).is_env_change());
        assert!(!GitFailure::Failed("x".into()).is_env_change());
        assert!(!GitFailure::NotARepo(std::path::PathBuf::from("/")).is_env_change());
    }

    #[test]
    fn test_file_entry_clone() {
        let entry = FileEntry {
            path: "test.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 5,
            deletions: 3,
        };
        let cloned = entry.clone();
        assert_eq!(cloned.path, "test.rs");
        assert_eq!(cloned.insertions, 5);
        assert_eq!(cloned.deletions, 3);
    }

    #[test]
    fn test_new_opens_gix_repo_on_valid_path() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        // repo_path should be populated
        assert!(!repo.repo_path.as_os_str().is_empty());
    }

    #[test]
    fn test_new_returns_not_a_repo_for_invalid_path() {
        let temp = std::env::temp_dir().join("git-rt-test-not-a-repo-task2");
        std::fs::create_dir_all(&temp).unwrap();
        let result = GitRepo::new(&temp);
        assert!(result.is_err());
        std::fs::remove_dir_all(&temp).ok();
    }
}
