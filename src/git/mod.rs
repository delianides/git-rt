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

/// Parsed diff output for a single file.
#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub hunks: Vec<DiffHunk>,
}

/// A single diff hunk.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A line within a diff hunk.
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

pub mod cli;
pub mod worker;
pub mod worktree;

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

/// Cached entry for `GitRepo::ahead_behind`: maps `(head_oid, upstream_oid)`
/// to the previously computed `(ahead, behind)` counts.
type AheadBehindCache =
    std::cell::RefCell<Option<((gix::ObjectId, gix::ObjectId), (usize, usize))>>;

/// Cached entry for `GitRepo::merge_base`: maps `(head_oid, base_ref)` to the
/// previously computed merge-base `ObjectId` (or `None` when none was found).
/// Caching `None` avoids re-walking when "no merge-base" is the answer.
type MergeBaseCache = std::cell::RefCell<Option<((gix::ObjectId, String), Option<gix::ObjectId>)>>;

/// Git repository handle backed by gix (gitoxide).
pub struct GitRepo {
    repo: gix::Repository,
    repo_path: PathBuf,
    /// Cache: (head_oid, upstream_oid) -> (ahead, behind).
    /// `ahead_behind()` is called per recompute; on a large repo each call
    /// walks the commit graph twice. Memoize keyed on the two input OIDs.
    /// `RefCell` is correct here — `GitRepo` is owned and accessed only on
    /// the single dedicated worker thread (no `Sync` needed).
    ahead_behind_cache: AheadBehindCache,
    /// Cache: (head_oid, base_ref) -> merge_base_oid.
    /// `merge_base()` builds a full topological index of HEAD's ancestry on
    /// every call, which is expensive on large repos. Memoize keyed on the
    /// (head_oid, base_ref) pair — invalidates correctly when either changes.
    merge_base_cache: MergeBaseCache,
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

        Ok(Self {
            repo,
            repo_path,
            ahead_behind_cache: std::cell::RefCell::new(None),
            merge_base_cache: std::cell::RefCell::new(None),
        })
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

    /// Compute the unified diff for a single file (worktree vs index).
    ///
    /// Tracked files go through `git diff -- <path>` for a real Myers diff.
    /// Untracked files fall back to a synthetic all-additions diff.
    pub fn diff_file(&self, path: &str) -> Result<FileDiff, GitFailure> {
        let work_dir = match self.repo.workdir() {
            Some(d) => d.to_path_buf(),
            None => return Ok(FileDiff::default()),
        };
        let worktree_path = work_dir.join(path);

        // Tracked? Shell out to `git diff -- <path>` for a real Myers diff.
        let index = match self.repo.index_or_empty() {
            Ok(i) => i,
            Err(e) => return Err(GitFailure::EnvChange(format!("diff_file index: {e}"))),
        };
        let path_bstr: &gix::bstr::BStr = path.as_bytes().into();
        if index.entry_by_path(path_bstr).is_some() {
            let bytes = crate::git::cli::run_diff_patch(&self.repo_path, None, path)?;
            return Ok(crate::git::cli::parse_unified_diff(&bytes));
        }

        // Not in the index: untracked (if present) or nothing.
        if worktree_path.exists() {
            self.diff_untracked(path)
                .map_err(|e| GitFailure::Failed(format!("diff_untracked: {e}")))
        } else {
            Ok(FileDiff::default())
        }
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
    ///
    /// The result is memoized by `(head_oid, upstream_oid)`: on a large repo
    /// each call previously walked the commit graph twice (O(N) where N is
    /// total commits). The cache is invalidated automatically whenever HEAD
    /// or upstream advances.
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

        // Cache hit: same (head, upstream) pair as last call — skip the walks.
        if let Some(((cached_head, cached_up), value)) = *self.ahead_behind_cache.borrow() {
            if cached_head == head_id && cached_up == upstream_id {
                return Ok(Some(value));
            }
        }

        // Compute ahead/behind by walking commits
        let ahead = count_reachable_exclusive(&self.repo, head_id, upstream_id)
            .map_err(|e| GitFailure::EnvChange(format!("ahead_behind ahead: {e}")))?;
        let behind = count_reachable_exclusive(&self.repo, upstream_id, head_id)
            .map_err(|e| GitFailure::EnvChange(format!("ahead_behind behind: {e}")))?;

        *self.ahead_behind_cache.borrow_mut() = Some(((head_id, upstream_id), (ahead, behind)));
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

    /// Compute the merge base between HEAD and the given base branch.
    ///
    /// For short names (no `/`), enumerates all plausible tips
    /// (`refs/heads/<name>` plus `refs/remotes/<remote>/<name>` for every
    /// configured remote) and returns the merge-base whose topological
    /// distance to HEAD is smallest. This keeps the branch view minimal
    /// even when local `<base>` is stale relative to `origin/<base>` (or
    /// vice versa) — the common case after `git fetch && git rebase
    /// origin/<base>`.
    ///
    /// For names containing `/` (e.g. `origin/release/x`, `feature/foo`),
    /// the name is treated as an exact ref and resolved with the
    /// single-tip path — the user's explicit intent wins.
    ///
    /// Returns `None` when:
    /// - The base name resolves to no tips,
    /// - The base equals the current branch (degenerate self-diff),
    /// - HEAD equals every candidate tip (fully behind every base),
    /// - No merge-base can be computed for any candidate.
    pub fn merge_base(&self, base_ref: &str) -> Result<Option<gix::ObjectId>, GitFailure> {
        let head_commit = self
            .repo
            .head_commit()
            .map_err(|e| GitFailure::EnvChange(format!("merge_base head: {e}")))?;
        let head_id = head_commit.id().detach();

        // Cache check: same (head_oid, base_ref) as last call → return cached value.
        if let Some(((cached_head, cached_base), value)) = self.merge_base_cache.borrow().as_ref() {
            if *cached_head == head_id && cached_base.as_str() == base_ref {
                return Ok(*value);
            }
        }

        // Compute the result, then write to cache unconditionally before returning.
        // All early-exit paths (degenerate self-diff, no tips, etc.) are also cached
        // so that repeated calls with the same inputs avoid re-walking.
        let result = self.merge_base_inner(head_id, base_ref);

        // Only cache successful computations; propagate errors directly.
        if let Ok(value) = result {
            *self.merge_base_cache.borrow_mut() = Some(((head_id, base_ref.to_owned()), value));
            return Ok(value);
        }

        result
    }

    /// Inner implementation of `merge_base` — called only on a cache miss.
    fn merge_base_inner(
        &self,
        head_id: gix::ObjectId,
        base_ref: &str,
    ) -> Result<Option<gix::ObjectId>, GitFailure> {
        // Self-as-base is degenerate (preserves existing semantics for
        // `merge_base(current_branch_name)` → `None`).
        if let Ok(name) = self.branch_name() {
            if name == base_ref {
                return Ok(None);
            }
        }

        // Names with `/` are exact refs — preserve explicit user intent.
        let tips: Vec<gix::ObjectId> = if base_ref.contains('/') {
            match self.resolve_ref_to_commit(base_ref) {
                Some(id) => vec![id],
                None => return Ok(None),
            }
        } else {
            let collected = self.collect_base_tips(base_ref);
            if collected.is_empty() {
                return Ok(None);
            }
            collected
        };

        // Build a topological index over HEAD's ancestry once. Smaller
        // index = closer to HEAD tip = smaller "what's on my branch" diff.
        let head_walk_index: std::collections::HashMap<gix::ObjectId, usize> = {
            let mut map = std::collections::HashMap::new();
            if let Ok(walk) = self.repo.rev_walk([head_id]).all() {
                for (idx, info) in walk.flatten().enumerate() {
                    map.insert(info.id, idx);
                }
            }
            map
        };

        // For each tip, compute merge-base; track the merge-base with the
        // smallest topological distance to HEAD.
        let mut best: Option<(usize, gix::ObjectId, gix::ObjectId)> = None; // (distance, mb, tip)
        for tip in &tips {
            // Skip tips that equal HEAD — they yield a degenerate merge-base
            // (HEAD itself) which is uninformative for branch view.
            if *tip == head_id {
                continue;
            }
            let mb = match self.find_merge_base(head_id, *tip) {
                Ok(Some(id)) => id,
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!(
                        target: "git_rt::git::base_resolve",
                        tip = %tip.to_hex(),
                        error = %e,
                        "merge-base walk failed for tip; skipping"
                    );
                    continue;
                }
            };
            if mb == head_id {
                continue;
            }
            let dist = head_walk_index.get(&mb).copied().unwrap_or(usize::MAX);
            match best {
                Some((d, _, _)) if d <= dist => {}
                _ => best = Some((dist, mb, *tip)),
            }
        }

        let chosen = best;
        let result = chosen.map(|(_, mb, _)| mb);
        tracing::debug!(
            target: "git_rt::git::base_resolve",
            name = base_ref,
            tip_count = tips.len(),
            chosen_tip = ?chosen.map(|(_, _, tip)| tip.to_hex().to_string()),
            chosen_mb = ?chosen.map(|(_, mb, _)| mb.to_hex().to_string()),
            topo_distance = ?chosen.map(|(d, _, _)| d),
            "resolved merge-base"
        );
        Ok(result)
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

    /// Enumerate all plausible commit tips for a short branch name.
    ///
    /// For `<name>`, returns the tips of `refs/heads/<name>` and
    /// `refs/remotes/<remote>/<name>` for every configured remote.
    /// Order: local first, then remotes in `remote_names()` order.
    /// Duplicates are deduplicated. Non-commit refs are skipped.
    fn collect_base_tips(&self, name: &str) -> Vec<gix::ObjectId> {
        let mut tips: Vec<gix::ObjectId> = Vec::new();

        let try_ref = |full_ref: &str, out: &mut Vec<gix::ObjectId>| {
            let Ok(reference) = self.repo.find_reference(full_ref) else {
                return;
            };
            let id = reference.id().detach();
            let Ok(obj) = self.repo.find_object(id) else {
                return;
            };
            if obj.kind == gix::object::Kind::Commit && !out.contains(&id) {
                out.push(id);
            }
        };

        try_ref(&format!("refs/heads/{name}"), &mut tips);
        for remote in self.repo.remote_names() {
            try_ref(
                &format!("refs/remotes/{}/{name}", remote.as_ref()),
                &mut tips,
            );
        }

        tips
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

    /// Compute the unified diff for a single file between the merge base
    /// and the current working tree. Tracked content goes through
    /// `git diff <merge_base> -- <path>` for a real Myers diff; untracked
    /// files (not in the merge-base tree, not in the index) fall back to
    /// the synthetic all-additions diff.
    pub fn branch_diff_file(
        &self,
        path: &str,
        merge_base: gix::ObjectId,
    ) -> Result<FileDiff, GitFailure> {
        let work_dir = match self.repo.workdir() {
            Some(d) => d.to_path_buf(),
            None => return Ok(FileDiff::default()),
        };
        let worktree_path = work_dir.join(path);

        // Is the file tracked in either the index or the merge-base tree?
        // If yes, let `git diff` handle every case (modified, deleted,
        // added-on-branch, binary).
        let in_index = match self.repo.index_or_empty() {
            Ok(idx) => {
                let path_bstr: &gix::bstr::BStr = path.as_bytes().into();
                idx.entry_by_path(path_bstr).is_some()
            }
            Err(e) => {
                return Err(GitFailure::EnvChange(format!(
                    "branch_diff_file index: {e}"
                )))
            }
        };

        let in_mb_tree = if !in_index {
            let mb_commit = self
                .repo
                .find_object(merge_base)
                .map_err(|e| GitFailure::EnvChange(format!("branch_diff_file find mb: {e}")))?
                .try_into_commit()
                .map_err(|e| GitFailure::EnvChange(format!("branch_diff_file into commit: {e}")))?;
            let mb_tree = mb_commit
                .tree()
                .map_err(|e| GitFailure::EnvChange(format!("branch_diff_file tree: {e}")))?;
            self.find_blob_in_tree(&mb_tree, path).is_some()
        } else {
            true
        };

        if in_index || in_mb_tree {
            let bytes = crate::git::cli::run_diff_patch(&self.repo_path, Some(&merge_base), path)?;
            return Ok(crate::git::cli::parse_unified_diff(&bytes));
        }

        // Not in index and not in merge-base tree: untracked addition.
        if worktree_path.exists() {
            self.diff_untracked(path)
                .map_err(|e| GitFailure::Failed(format!("branch_diff_file untracked: {e}")))
        } else {
            Ok(FileDiff::default())
        }
    }

    fn find_blob_in_tree(&self, tree: &gix::Tree<'_>, path: &str) -> Option<String> {
        let parts: Vec<&str> = path.split('/').collect();
        self.walk_tree_for_blob(tree, &parts)
    }

    fn walk_tree_for_blob(&self, tree: &gix::Tree<'_>, parts: &[&str]) -> Option<String> {
        use gix::bstr::ByteSlice;

        if parts.is_empty() {
            return None;
        }

        let first = parts[0];
        let rest = &parts[1..];

        for entry in tree.iter().flatten() {
            let name = entry.filename().to_str_lossy();
            if name == first {
                if rest.is_empty() {
                    // Leaf: should be a blob
                    let id = entry.object_id();
                    let obj = self.repo.find_object(id).ok()?;
                    if obj.kind.is_blob() {
                        return Some(String::from_utf8_lossy(&obj.data).into_owned());
                    }
                    return None;
                } else {
                    // Descend into subtree
                    let id = entry.object_id();
                    let obj = self.repo.find_object(id).ok()?;
                    let subtree = obj.try_into_tree().ok()?;
                    return self.walk_tree_for_blob(&subtree, rest);
                }
            }
        }
        None
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
    fn test_diff_file_handles_missing_path() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let result = repo.diff_file("nonexistent-file-xyz-12345.rs");
        assert!(result.is_ok());
        let diff = result.unwrap();
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
    fn ahead_behind_cache_populated_after_call() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let r1 = repo.ahead_behind().ok().flatten();
        // After the first call the cache should be populated iff we got Some.
        {
            let cached = repo.ahead_behind_cache.borrow();
            if r1.is_some() {
                assert!(
                    cached.is_some(),
                    "cache must be populated when ahead_behind returned Some"
                );
            }
        }
        // Second call should return the same value (from cache or fresh).
        let r2 = repo.ahead_behind().ok().flatten();
        assert_eq!(
            r1, r2,
            "repeated ahead_behind calls must return equal results"
        );
    }

    #[test]
    fn merge_base_cache_populated_after_call() {
        let repo = GitRepo::new(std::path::Path::new(".")).unwrap();
        let r1 = repo.merge_base("main").ok();
        assert!(
            repo.merge_base_cache.borrow().is_some(),
            "merge_base cache should be populated after a call"
        );
        let r2 = repo.merge_base("main").ok();
        assert_eq!(r1, r2, "second call should return same result");
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
    fn merge_base_picks_remote_when_local_stale() {
        // Simulates `git fetch && git rebase origin/main` on a feature branch.
        // Local main is stale (at C1); origin/main is current (at C3); feature
        // is rebased onto C3. The closest merge-base must be C3, not C1.
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        init_repo_for_discover(&repo_path);

        // Build C1 → C2 → C3 on main.
        std::fs::write(repo_path.join("a.txt"), "a").unwrap();
        git(&repo_path, &["add", "a.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C1"]);
        let c1 = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

        std::fs::write(repo_path.join("b.txt"), "b").unwrap();
        git(&repo_path, &["add", "b.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C2"]);

        std::fs::write(repo_path.join("c.txt"), "c").unwrap();
        git(&repo_path, &["add", "c.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C3"]);
        let c3 = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let c3 = String::from_utf8(c3.stdout).unwrap().trim().to_string();

        // Move local main pointer back to C1 (simulating "stale local main").
        git(&repo_path, &["update-ref", "refs/heads/main", &c1]);

        // Plant origin/main at C3 (simulating "fetched but didn't pull local").
        git(&repo_path, &["update-ref", "refs/remotes/origin/main", &c3]);
        // Add a remote so remote_names() includes "origin".
        git(&repo_path, &["remote", "add", "origin", "."]);

        // Create feature off C3 with one feature commit.
        git(&repo_path, &["checkout", "-q", "-b", "feature", &c3]);
        std::fs::write(repo_path.join("f.txt"), "f").unwrap();
        git(&repo_path, &["add", "f.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "F1"]);

        let repo = GitRepo::new(&repo_path).unwrap();
        let mb = repo.merge_base("main").unwrap().unwrap();
        assert_eq!(
            mb.to_hex().to_string(),
            c3,
            "merge-base must be C3 (origin/main), not C1 (stale local main)",
        );
    }

    #[test]
    fn merge_base_skips_tip_equal_to_head() {
        // User is on `feature`, just branched from `main` with no new commits.
        // `collect_base_tips("main")` returns [local_main_tip] (== HEAD).
        // The loop must skip that tip; with no other tips, result is None.
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        init_repo_for_discover(&repo_path);

        // Single commit on main; that's where feature will branch from.
        std::fs::write(repo_path.join("a.txt"), "a").unwrap();
        git(&repo_path, &["add", "a.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C1"]);

        // Branch feature off main with no extra commits — HEAD == main's tip.
        git(&repo_path, &["checkout", "-q", "-b", "feature"]);

        let repo = GitRepo::new(&repo_path).unwrap();
        let result = repo.merge_base("main").unwrap();
        assert!(
            result.is_none(),
            "merge_base must be None when the only candidate tip equals HEAD; got {:?}",
            result.map(|id| id.to_hex().to_string())
        );
    }

    #[test]
    fn merge_base_picks_local_when_remote_stale() {
        // Local main is current (at C3); origin/main is stale (at C1).
        // Feature is off C3. The closest merge-base must be C3, not C1.
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        init_repo_for_discover(&repo_path);

        std::fs::write(repo_path.join("a.txt"), "a").unwrap();
        git(&repo_path, &["add", "a.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C1"]);
        let c1 = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo_path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        std::fs::write(repo_path.join("b.txt"), "b").unwrap();
        git(&repo_path, &["add", "b.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C2"]);

        std::fs::write(repo_path.join("c.txt"), "c").unwrap();
        git(&repo_path, &["add", "c.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C3"]);
        let c3 = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo_path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        // Plant origin/main at C1 (stale remote).
        git(&repo_path, &["update-ref", "refs/remotes/origin/main", &c1]);
        git(&repo_path, &["remote", "add", "origin", "."]);

        // Feature off C3.
        git(&repo_path, &["checkout", "-q", "-b", "feature", &c3]);
        std::fs::write(repo_path.join("f.txt"), "f").unwrap();
        git(&repo_path, &["add", "f.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "F1"]);

        let repo = GitRepo::new(&repo_path).unwrap();
        let mb = repo.merge_base("main").unwrap().unwrap();
        assert_eq!(
            mb.to_hex().to_string(),
            c3,
            "merge-base must be C3 (local main, current), not C1 (stale origin/main)",
        );
    }

    #[test]
    fn merge_base_handles_divergence() {
        // C1 is the common ancestor.
        // Local main: C1 → L (diverged left).
        // origin/main: C1 → R (diverged right).
        // Feature is off R with one commit. The merge-base of HEAD with R
        // (which equals R) is closer to HEAD than the merge-base of HEAD with
        // L (which is C1).
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        init_repo_for_discover(&repo_path);

        std::fs::write(repo_path.join("a.txt"), "a").unwrap();
        git(&repo_path, &["add", "a.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C1"]);
        let c1 = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo_path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        // Local main advances to L.
        std::fs::write(repo_path.join("l.txt"), "l").unwrap();
        git(&repo_path, &["add", "l.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "L"]);

        // Build R off C1 and plant it as origin/main.
        git(&repo_path, &["checkout", "-q", "-b", "tmp-r", &c1]);
        std::fs::write(repo_path.join("r.txt"), "r").unwrap();
        git(&repo_path, &["add", "r.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "R"]);
        let r = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo_path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        git(&repo_path, &["update-ref", "refs/remotes/origin/main", &r]);
        git(&repo_path, &["remote", "add", "origin", "."]);

        // Feature off R with one commit.
        git(&repo_path, &["checkout", "-q", "-b", "feature", &r]);
        std::fs::write(repo_path.join("f.txt"), "f").unwrap();
        git(&repo_path, &["add", "f.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "F1"]);

        let repo = GitRepo::new(&repo_path).unwrap();
        let mb = repo.merge_base("main").unwrap().unwrap();
        assert_eq!(
            mb.to_hex().to_string(),
            r,
            "merge-base must be R (closer to HEAD), not C1 (local main's mb)",
        );
    }

    #[test]
    fn merge_base_exact_ref_with_slash_skips_enumeration() {
        // Setup: main with one commit, origin/release/x at that commit, then
        // feature with one extra commit. Calling merge_base("origin/release/x")
        // must resolve to the planted ref directly and produce the planted commit
        // as the merge-base.
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        init_repo_for_discover(&repo_path);

        std::fs::write(repo_path.join("a.txt"), "a").unwrap();
        git(&repo_path, &["add", "a.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "C1"]);
        let c1 = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo_path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        git(
            &repo_path,
            &["update-ref", "refs/remotes/origin/release/x", &c1],
        );
        git(&repo_path, &["remote", "add", "origin", "."]);

        git(&repo_path, &["checkout", "-q", "-b", "feature", &c1]);
        std::fs::write(repo_path.join("f.txt"), "f").unwrap();
        git(&repo_path, &["add", "f.txt"]);
        git(&repo_path, &["commit", "-q", "-m", "F1"]);

        let repo = GitRepo::new(&repo_path).unwrap();
        let mb = repo.merge_base("origin/release/x").unwrap().unwrap();
        assert_eq!(
            mb.to_hex().to_string(),
            c1,
            "merge-base for exact slash-name must equal the planted commit",
        );
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

    /// Regression test for the bug where a small non-contiguous edit was
    /// rendered as one giant delete-then-add block. The fix shells out to
    /// `git diff -p` and parses the unified-diff output, so changes at the
    /// top and bottom of a file produce TWO hunks, not one.
    #[test]
    fn diff_file_produces_multiple_hunks_for_non_contiguous_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo_for_discover(repo);

        // Write a 30-line file and commit it.
        let original: String = (1..=30)
            .map(|i| format!("line {i}\n"))
            .collect::<Vec<_>>()
            .join("");
        std::fs::write(repo.join("file.txt"), &original).unwrap();
        git(repo, &["add", "file.txt"]);
        git(repo, &["commit", "-q", "-m", "add file"]);

        // Modify line 2 and line 28 — separate, non-contiguous changes.
        let modified: String = (1..=30)
            .map(|i| match i {
                2 => "line 2 CHANGED\n".to_string(),
                28 => "line 28 CHANGED\n".to_string(),
                _ => format!("line {i}\n"),
            })
            .collect::<Vec<_>>()
            .join("");
        std::fs::write(repo.join("file.txt"), &modified).unwrap();

        let r = GitRepo::new(repo).unwrap();
        let diff = r.diff_file("file.txt").expect("diff_file ok");

        assert!(
            diff.hunks.len() >= 2,
            "expected at least 2 hunks for non-contiguous changes, got {}: {:?}",
            diff.hunks.len(),
            diff.hunks.iter().map(|h| &h.header).collect::<Vec<_>>()
        );

        let dels = diff
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| matches!(l.kind, DiffLineKind::Deletion))
            .count();
        let adds = diff
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| matches!(l.kind, DiffLineKind::Addition))
            .count();
        assert_eq!(dels, 2, "two lines deleted");
        assert_eq!(adds, 2, "two lines added");
    }

    /// Branch view variant of the previous test: changes committed on a
    /// branch since merge-base must also render as multiple hunks.
    #[test]
    fn branch_diff_file_produces_multiple_hunks_for_non_contiguous_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo_for_discover(repo);

        let original: String = (1..=30)
            .map(|i| format!("line {i}\n"))
            .collect::<Vec<_>>()
            .join("");
        std::fs::write(repo.join("file.txt"), &original).unwrap();
        git(repo, &["add", "file.txt"]);
        git(repo, &["commit", "-q", "-m", "add file"]);

        // Branch off main and commit a non-contiguous change.
        git(repo, &["checkout", "-q", "-b", "feature"]);
        let modified: String = (1..=30)
            .map(|i| match i {
                2 => "line 2 CHANGED\n".to_string(),
                28 => "line 28 CHANGED\n".to_string(),
                _ => format!("line {i}\n"),
            })
            .collect::<Vec<_>>()
            .join("");
        std::fs::write(repo.join("file.txt"), &modified).unwrap();
        git(repo, &["add", "file.txt"]);
        git(repo, &["commit", "-q", "-m", "edit"]);

        let r = GitRepo::new(repo).unwrap();
        let mb = r.merge_base("main").unwrap().expect("merge base");
        let diff = r
            .branch_diff_file("file.txt", mb)
            .expect("branch_diff_file ok");

        assert!(
            diff.hunks.len() >= 2,
            "expected at least 2 hunks, got {}: {:?}",
            diff.hunks.len(),
            diff.hunks.iter().map(|h| &h.header).collect::<Vec<_>>()
        );
    }
}
