use std::path::{Path, PathBuf};

use anyhow::Result;
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

/// Errors from `discover_worktree_root`.
#[derive(Debug, Error)]
pub enum DiscoverError {
    #[error("Not inside a git repository: {0}")]
    NotInRepo(PathBuf),
    #[error("Bare repositories have no working tree: {0}")]
    BareRepo(PathBuf),
}

/// Discover the working-tree root for a path inside a git repository.
///
/// Walks upward from `start` to find the enclosing `.git` directory or file.
/// For a path inside a linked worktree, returns that worktree's root (not the
/// main worktree's). Returns `Err` if `start` is not inside a repo, or if the
/// discovered repo is bare (no working tree).
pub fn discover_worktree_root(start: &Path) -> Result<PathBuf, DiscoverError> {
    let repo = gix::discover(start).map_err(|_| DiscoverError::NotInRepo(start.to_path_buf()))?;
    match repo.workdir() {
        Some(wd) => Ok(wd.to_path_buf()),
        None => Err(DiscoverError::BareRepo(start.to_path_buf())),
    }
}

pub mod cli;
pub mod worker;

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

/// Resolve the filesystem path of the main worktree for any repo path.
///
/// For a linked worktree, this is the parent of the common gitdir
/// (the canonical repo checkout). For a standard repo, it is `repo_path`.
/// Falls back to `repo_path` if the common gitdir has no parent (bare repo
/// or unusual layout).
pub fn main_worktree_path(repo_path: &Path) -> PathBuf {
    let common_git_dir =
        resolve_common_git_dir(repo_path).unwrap_or_else(|| repo_path.join(".git"));
    common_git_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_path.to_path_buf())
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
        // use filesystem paths (e.g., repo_name/worktree_name which take file_name of the path).
        //
        // gix's workdir() may return a relative path (e.g., "."). We canonicalize
        // it to ensure .file_name() and .parent() work correctly — a relative
        // path like "." has no file_name.
        let repo_path = repo
            .workdir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf());
        let repo_path = std::fs::canonicalize(&repo_path).unwrap_or(repo_path);

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

    /// Compute the current status of all changed files with numstat,
    /// relative to HEAD (no base branch). Delegates to `git status --porcelain=v2`
    /// + `git diff --numstat` via [`crate::git::cli::compute_status_files`].
    pub fn status(&self) -> Result<Vec<FileEntry>, GitFailure> {
        crate::git::cli::compute_status_files(&self.repo_path, None)
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
            .workdir()
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

    /// Count the number of stash entries.
    /// Stashes are stored as reflog entries on refs/stash.
    pub fn stash_count(&self) -> Result<usize, GitFailure> {
        // Try to find the stash ref. If missing, return 0.
        let stash_ref = match self.repo.find_reference("refs/stash") {
            Ok(r) => r,
            Err(_) => return Ok(0), // no stash ref = no stashes
        };

        // Count reflog entries
        match stash_ref.log_iter().all() {
            Ok(Some(iter)) => Ok(iter.count()),
            Ok(None) => Ok(0),
            Err(e) => Err(GitFailure::EnvChange(format!("stash_count: {e}"))),
        }
    }

    /// Get ahead/behind counts relative to upstream.
    /// Returns None if there is no upstream configured.
    pub fn ahead_behind(&self) -> Result<Option<(usize, usize)>, GitFailure> {
        // Get HEAD commit id
        let head_commit = match self.repo.head_commit() {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        let head_id = head_commit.id().detach();

        // Resolve upstream commit id
        let upstream_id = match self.upstream_commit_id() {
            Some(id) => id,
            None => return Ok(None), // no upstream configured
        };

        // Compute ahead/behind by walking commits
        let ahead = count_reachable_exclusive(&self.repo, head_id, upstream_id)
            .map_err(|e| GitFailure::EnvChange(format!("ahead_behind ahead: {e}")))?;
        let behind = count_reachable_exclusive(&self.repo, upstream_id, head_id)
            .map_err(|e| GitFailure::EnvChange(format!("ahead_behind behind: {e}")))?;

        Ok(Some((ahead, behind)))
    }

    /// Try to find the commit id of the current branch's upstream.
    fn upstream_commit_id(&self) -> Option<gix::ObjectId> {
        // Get current branch short name
        let head_name = self.repo.head_name().ok().flatten()?;
        let short = head_name.shorten().to_string();

        // Look up branch config
        let config = self.repo.config_snapshot();
        let remote_key = format!("branch.{short}.remote");
        let merge_key = format!("branch.{short}.merge");

        let remote = config.string(remote_key.as_str())?;
        let merge_ref = config.string(merge_key.as_str())?;

        // merge_ref is "refs/heads/<branch>", strip to get branch name
        let merge_branch = merge_ref.strip_prefix(b"refs/heads/" as &[u8])?;

        // Construct tracking ref: refs/remotes/<remote>/<branch>
        let mut tracking = Vec::new();
        tracking.extend_from_slice(b"refs/remotes/");
        tracking.extend_from_slice(remote.as_ref());
        tracking.push(b'/');
        tracking.extend_from_slice(merge_branch);
        let tracking_str = std::str::from_utf8(&tracking).ok()?;

        // Resolve the tracking ref to a commit id
        let reference = self.repo.find_reference(tracking_str).ok()?;
        // Peel to commit (in case it's a tag or symbolic ref)
        let id = reference.id();
        Some(id.detach())
    }

    /// Compute the merge base between HEAD and the given base ref.
    ///
    /// Returns `None` if the ref can't be resolved, if HEAD equals the merge base
    /// (i.e., the current branch is fully behind the base), or if HEAD is detached
    /// and equals the base commit.
    pub fn merge_base(&self, base_ref: &str) -> Result<Option<gix::ObjectId>, GitFailure> {
        let base_id = self.resolve_ref_to_commit(base_ref);
        let base_id = match base_id {
            Some(id) => id,
            None => return Ok(None),
        };

        let head_commit = match self.repo.head_commit() {
            Ok(c) => c,
            Err(e) => return Err(GitFailure::EnvChange(format!("merge_base head: {e}"))),
        };
        let head_id = head_commit.id().detach();

        if head_id == base_id {
            return Ok(None);
        }

        let base = self
            .find_merge_base(head_id, base_id)
            .map_err(|e| GitFailure::EnvChange(format!("merge_base walk: {e}")))?;

        match base {
            Some(mb) if mb == head_id => Ok(None),
            other => Ok(other),
        }
    }

    /// Try to resolve a ref name to a commit ObjectId.
    ///
    /// Tries multiple candidate forms: as-is, "origin/<name>",
    /// "refs/remotes/origin/<name>", and "refs/heads/<name>".
    fn resolve_ref_to_commit(&self, name: &str) -> Option<gix::ObjectId> {
        let candidates = [
            name.to_string(),
            format!("origin/{name}"),
            format!("refs/remotes/origin/{name}"),
            format!("refs/heads/{name}"),
        ];

        for candidate in &candidates {
            if let Ok(reference) = self.repo.find_reference(candidate.as_str()) {
                let id = reference.id().detach();
                if let Ok(obj) = self.repo.find_object(id) {
                    if obj.kind == gix::object::Kind::Commit {
                        return Some(id);
                    }
                }
            }
        }

        None
    }

    /// Find the merge base (most recent common ancestor) of two commits.
    ///
    /// TODO: This collects all ancestors of `b` into memory before walking `a`.
    /// For repos with very long histories (100k+ commits), consider using an
    /// interleaved BFS or gix's built-in merge-base support for better perf.
    fn find_merge_base(
        &self,
        a: gix::ObjectId,
        b: gix::ObjectId,
    ) -> Result<Option<gix::ObjectId>, Box<dyn std::error::Error + Send + Sync>> {
        let b_ancestors: std::collections::HashSet<gix::ObjectId> = {
            let mut set = std::collections::HashSet::new();
            let walk = self.repo.rev_walk([b]).all()?;
            for info in walk {
                let info = info?;
                set.insert(info.id);
            }
            set
        };

        let walk = self.repo.rev_walk([a]).all()?;
        for info in walk {
            let info = info?;
            if b_ancestors.contains(&info.id) {
                return Ok(Some(info.id));
            }
        }

        Ok(None)
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

    /// Resolve the base branch name using priority:
    /// 1. Explicit override (CLI flag or config value, pre-merged by caller)
    /// 2. Auto-detect from origin/HEAD
    /// 3. Fallback to origin/main, then origin/master
    pub fn resolve_base_branch(&self, explicit_base: Option<&str>) -> Option<String> {
        if let Some(base) = explicit_base {
            return Some(base.to_string());
        }

        // Priority 2: resolve origin/HEAD symbolic ref to its target.
        // gix's symbolic ref API can be tricky, so read the file directly.
        let origin_head_path = self.repo.git_dir().join("refs/remotes/origin/HEAD");
        if let Ok(content) = std::fs::read_to_string(&origin_head_path) {
            if let Some(target) = content.strip_prefix("ref: refs/remotes/origin/") {
                return Some(target.trim().to_string());
            }
        }

        // Priority 4: fallback
        if self
            .resolve_ref_to_commit("refs/remotes/origin/main")
            .is_some()
        {
            return Some("main".to_string());
        }
        if self
            .resolve_ref_to_commit("refs/remotes/origin/master")
            .is_some()
        {
            return Some("master".to_string());
        }

        None
    }

    /// Compute the file list for the branch view: union of committed changes
    /// (vs `merge_base`) and uncommitted changes (vs HEAD), with untracked
    /// files included.
    ///
    /// Delegates to `git diff --numstat <merge_base>` + `git status --porcelain=v2`
    /// via [`crate::git::cli::compute_status_files`].
    pub fn branch_status(&self, merge_base: gix::ObjectId) -> Result<Vec<FileEntry>, GitFailure> {
        crate::git::cli::compute_status_files(&self.repo_path, Some(&merge_base))
    }
}

/// Count commits reachable from `from` but not from `exclude`.
fn count_reachable_exclusive(
    repo: &gix::Repository,
    from: gix::ObjectId,
    exclude: gix::ObjectId,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Walk all ancestors of `exclude` into a hash set
    let excluded: std::collections::HashSet<gix::ObjectId> = {
        let mut set = std::collections::HashSet::new();
        let walk = repo.rev_walk([exclude]).all()?;
        for info in walk {
            let info = info?;
            set.insert(info.id);
        }
        set
    };

    // Walk ancestors of `from`, counting those not in `excluded`
    let mut count = 0;
    let walk = repo.rev_walk([from]).all()?;
    for info in walk {
        let info = info?;
        if !excluded.contains(&info.id) {
            count += 1;
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_status_works_against_real_repo() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let result = repo.status();
        assert!(result.is_ok(), "status() should succeed on a valid repo");
    }

    #[test]
    fn test_status_returns_modified_file() {
        // Uses the worktree itself — if there are staged or unstaged
        // changes, at least one entry should be returned. If the tree is
        // clean this test is a no-op assertion.
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let entries = repo.status().unwrap();
        for e in &entries {
            // Paths must be relative (not absolute) and non-empty.
            assert!(!e.path.is_empty());
            assert!(!e.path.starts_with('/'));
        }
    }

    #[test]
    fn test_merge_base_returns_none_on_same_branch() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let branch = repo.branch_name().unwrap();
        let result = repo.merge_base(&branch);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_merge_base_returns_none_for_nonexistent_ref() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let result = repo.merge_base("nonexistent-branch-xyz-99999");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_new_returns_not_a_repo_for_invalid_path() {
        let temp = std::env::temp_dir().join("git-rt-test-not-a-repo-task2");
        std::fs::create_dir_all(&temp).unwrap();
        let result = GitRepo::new(&temp);
        assert!(result.is_err());
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_resolve_base_branch_with_explicit() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let result = repo.resolve_base_branch(Some("main"));
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_resolve_base_branch_none_when_no_remote() {
        let dir = std::env::temp_dir().join("git-rt-test-no-remote");
        std::fs::create_dir_all(&dir).ok();
        let result = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&dir)
            .output();
        if result.is_ok() {
            if let Ok(repo) = GitRepo::new(&dir) {
                let result = repo.resolve_base_branch(None);
                assert!(result.is_none(), "should be None with no remote");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_branch_status_returns_entries() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();

        if let Some(base) = repo.resolve_base_branch(None) {
            if let Ok(Some(mb)) = repo.merge_base(&base) {
                let result = repo.branch_status(mb);
                assert!(result.is_ok());
                let entries = result.unwrap();
                for entry in &entries {
                    assert!(!entry.path.is_empty());
                    assert!(!entry.path.starts_with('/'));
                }
            }
        }
    }

    #[test]
    fn test_main_worktree_path_for_linked_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main-repo");
        let worktrees = main.join(".git").join("worktrees").join("feat");
        let linked = tmp.path().join("linked-wt");
        std::fs::create_dir_all(&worktrees).unwrap();
        std::fs::create_dir_all(&linked).unwrap();

        // .git file in the linked worktree points into main/.git/worktrees/feat
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", worktrees.display()),
        )
        .unwrap();
        // commondir file resolves back to the main gitdir
        std::fs::write(worktrees.join("commondir"), "../..\n").unwrap();
        std::fs::create_dir_all(main.join(".git")).unwrap();

        let result = main_worktree_path(&linked);
        // Canonicalize both sides since resolve_common_git_dir canonicalizes
        // the result.
        assert_eq!(
            std::fs::canonicalize(result).unwrap(),
            std::fs::canonicalize(&main).unwrap(),
        );
    }

    #[test]
    fn test_main_worktree_path_for_standard_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        let result = main_worktree_path(&repo);
        assert_eq!(result, repo);
    }

    // --- discover_worktree_root tests ---

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
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

    fn init_repo_for_discover(path: &std::path::Path) {
        std::fs::create_dir_all(path).unwrap();
        git(path, &["init", "-q", "-b", "main"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["config", "user.name", "Test"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        git(path, &["commit", "--allow-empty", "-q", "-m", "init"]);
    }

    #[test]
    fn discover_worktree_root_from_root() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo_for_discover(&repo);

        let result = discover_worktree_root(&repo).unwrap();
        assert_eq!(result.canonicalize().unwrap(), repo.canonicalize().unwrap());
    }

    #[test]
    fn discover_worktree_root_from_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo_for_discover(&repo);
        let nested = repo.join("src").join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        let result = discover_worktree_root(&nested).unwrap();
        assert_eq!(result.canonicalize().unwrap(), repo.canonicalize().unwrap());
    }

    #[test]
    fn discover_worktree_root_not_in_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let not_a_repo = tmp.path().join("empty");
        std::fs::create_dir_all(&not_a_repo).unwrap();

        let err = discover_worktree_root(&not_a_repo).unwrap_err();
        match err {
            DiscoverError::NotInRepo(p) => {
                assert_eq!(
                    p.canonicalize().unwrap(),
                    not_a_repo.canonicalize().unwrap()
                );
            }
            other => panic!("expected NotInRepo, got {other:?}"),
        }
    }

    #[test]
    fn discover_worktree_root_from_linked_worktree_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("repo");
        init_repo_for_discover(&main);

        let linked = main.join(".worktrees").join("feat");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat",
                linked.to_str().unwrap(),
            ],
        );
        let nested = linked.join("sub").join("dir");
        std::fs::create_dir_all(&nested).unwrap();

        let result = discover_worktree_root(&nested).unwrap();
        assert_eq!(
            result.canonicalize().unwrap(),
            linked.canonicalize().unwrap(),
            "should return the linked worktree's root, not the main worktree's"
        );
    }

    #[test]
    fn discover_worktree_root_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("bare.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let err = discover_worktree_root(&bare).unwrap_err();
        assert!(
            matches!(err, DiscoverError::BareRepo(_)),
            "expected BareRepo, got {err:?}"
        );
    }
}
