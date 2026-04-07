use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("Not a git repository: {0}")]
    NotARepo(PathBuf),
    #[error("Failed to compute status: {0}")]
    StatusFailed(String),
    #[error("Failed to compute diff: {0}")]
    DiffFailed(String),
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

/// Git repository handle
pub struct GitRepo {
    repo_path: PathBuf,
    // TODO: Replace with gix::Repository once we wire up gitoxide properly.
    // For the initial scaffold we shell out to git for correctness,
    // then migrate to gix for performance.
    //
    // gix integration plan:
    //   let repo = gix::open(&repo_path)?;
    //   let status = repo.status(...)? for file listing
    //   let diff = repo.diff_tree_to_workdir(...)? for diffs
}

impl GitRepo {
    pub fn new(path: &Path) -> Result<Self> {
        // Verify this is a git repo
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output()
            .context("Failed to run git")?;

        if !output.status.success() {
            return Err(GitError::NotARepo(path.to_path_buf()).into());
        }

        let repo_root = String::from_utf8(output.stdout)
            .context("Invalid UTF-8 in git output")?
            .trim()
            .to_string();

        Ok(Self {
            repo_path: PathBuf::from(repo_root),
        })
    }

    /// Get the current branch name, or "HEAD" if detached
    pub fn branch_name(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git rev-parse")?;

        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(if name.is_empty() { "HEAD".to_string() } else { name })
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

        // Get numstat for insertion/deletion counts
        let numstat_output = Command::new("git")
            .args(["diff", "--numstat"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git diff --numstat")?;

        let numstat_str = String::from_utf8_lossy(&numstat_output.stdout);

        // Build a map of path -> (insertions, deletions)
        let mut stats: std::collections::HashMap<String, (usize, usize)> =
            std::collections::HashMap::new();

        for line in numstat_str.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let ins = parts[0].parse::<usize>().unwrap_or(0);
                let del = parts[1].parse::<usize>().unwrap_or(0);
                let path = parts[2].to_string();
                stats.insert(path, (ins, del));
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
}
