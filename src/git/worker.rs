//! Async git worker thread.
//!
//! Owns the [`GitRepo`] and processes git requests on a dedicated thread,
//! keeping the UI event loop responsive regardless of how slow individual
//! git operations are. Communication is via two crossbeam channels:
//! the main thread sends [`Request`] messages and receives [`Response`]
//! messages.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender};

use crate::git::{FileDiff, FileEntry, GitRepo};

/// Token used to discard stale diff results — `handle_expand` increments
/// the next-token counter and stamps each `Request::Diff`. When the worker
/// echoes the token back in `Response::Diff`, the main thread compares
/// against the current pending token and drops any result that doesn't
/// match (user moved selection, closed overlay, switched worktrees).
pub type DiffToken = u64;

/// Per-branch cache of detected base branch names. Owned by the worker
/// run-loop; cleared on `SwitchRepo`. Stores `Option<String>` so a
/// genuine "no base detectable" result is remembered and doesn't retrigger
/// detection on every recompute.
#[derive(Debug, Default)]
pub(crate) struct BaseCache {
    entries: HashMap<String, Option<String>>,
}

impl BaseCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get(&self, branch: &str) -> Option<&Option<String>> {
        self.entries.get(branch)
    }

    pub(crate) fn insert(&mut self, branch: String, base: Option<String>) {
        self.entries.insert(branch, base);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    // Used by tests (compute_status_with_explicit_override_skips_cache)
    // to assert the explicit-override path doesn't populate the cache.
    // clippy's lib-target dead-code analysis does not count test-only usage.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Requests the main thread sends to the worker.
#[derive(Debug)]
pub enum Request {
    /// Recompute status + branch metadata. Coalesced — only the most recent
    /// pending Recompute is kept when the worker drains its channel.
    Recompute,
    /// Compute the diff for a single file. `token` lets the receiver
    /// discard stale results.
    Diff { path: String, token: DiffToken },
    /// Re-open the worker's `GitRepo` against a new path.
    /// Worker replies with `Response::SwitchAck` once the new repo is open.
    SwitchRepo(PathBuf),
    /// Stop the worker thread. Worker exits its loop after handling.
    Shutdown,
}

/// Responses the worker sends back to the main thread.
#[derive(Debug)]
pub enum Response {
    /// Result of a `Recompute` request.
    Status(Box<StatusBundle>),
    /// Result of a `Diff` request. `token` echoes the request's token.
    Diff {
        path: String,
        token: DiffToken,
        diff: FileDiff,
    },
    /// Sent after a `SwitchRepo` request finishes (success or failure).
    /// The bool is `true` on success, `false` on failure (worker keeps
    /// the previous repo so the app stays usable).
    SwitchAck(bool),
    /// Worker hit a non-fatal error processing a request. The main thread
    /// can log it and decide whether to surface it.
    Error(String),
}

/// All the git-derived state a `Recompute` returns. Mirrors what the old
/// synchronous `handle_fs_change` populated on `AppState`.
#[derive(Debug, Default)]
pub struct StatusBundle {
    pub files: Vec<FileEntry>,
    pub merge_base: Option<gix::ObjectId>,
    pub base_branch: String,
    pub branch: String,
    pub head: Option<(String, String)>,
    pub stash_count: usize,
    pub ahead_behind: Option<(usize, usize)>,
    pub repo_state: Option<String>,
    pub repo_name: String,
    pub worktree_name: String,
}

/// Coalesce a batch of pending requests so redundant `Recompute` messages
/// collapse to a single one. `Diff`, `SwitchRepo`, and `Shutdown` are
/// preserved in FIFO order. Pure function — no I/O, no thread access.
///
/// Behavior:
/// - Multiple `Recompute` → exactly one `Recompute`, **always appended at
///   the end** of the output queue. All `Diff` and `SwitchRepo` requests
///   are therefore processed before the next status sweep.
/// - Other variants → preserved in original FIFO order.
/// - Empty input → empty output.
pub fn coalesce(input: VecDeque<Request>) -> VecDeque<Request> {
    let mut has_recompute = false;
    let mut output: VecDeque<Request> = VecDeque::with_capacity(input.len());

    for req in input {
        if matches!(req, Request::Recompute) {
            has_recompute = true;
        } else {
            output.push_back(req);
        }
    }

    if has_recompute {
        output.push_back(Request::Recompute);
    }

    output
}

/// The worker thread harness. Owns a `GitRepo`, processes `Request`s, and
/// sends `Response`s back. Created via [`Worker::spawn`].
pub struct Worker;

impl Worker {
    /// Spawn the worker thread. Returns its `JoinHandle`. The thread runs
    /// until a `Shutdown` request is received OR the request channel is
    /// dropped.
    pub fn spawn(
        repo_path: PathBuf,
        base_override: Option<String>,
        config_base: Option<String>,
        req_rx: Receiver<Request>,
        resp_tx: Sender<Response>,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name("git-worker".to_string())
            .spawn(move || {
                Self::run(repo_path, base_override, config_base, req_rx, resp_tx);
            })
            .expect("failed to spawn git-worker thread")
    }

    fn run(
        repo_path: PathBuf,
        base_override: Option<String>,
        config_base: Option<String>,
        req_rx: Receiver<Request>,
        resp_tx: Sender<Response>,
    ) {
        // Open the repo. If it fails, send Error and exit.
        let mut git = match GitRepo::new(&repo_path) {
            Ok(g) => g,
            Err(e) => {
                let _ = resp_tx.send(Response::Error(format!("worker open: {e}")));
                return;
            }
        };

        let mut cache = BaseCache::new();

        loop {
            // Block on the next request.
            let first = match req_rx.recv() {
                Ok(r) => r,
                Err(_) => return, // channel closed
            };

            // Drain backlog into a queue, prepending `first`.
            let mut queue: VecDeque<Request> = VecDeque::new();
            queue.push_back(first);
            while let Ok(more) = req_rx.try_recv() {
                queue.push_back(more);
            }

            // Coalesce.
            let queue = coalesce(queue);

            // Process in order.
            for req in queue {
                match req {
                    Request::Recompute => {
                        let bundle = compute_status(
                            &git,
                            base_override.as_deref(),
                            config_base.as_deref(),
                            &mut cache,
                        );
                        let _ = resp_tx.send(Response::Status(Box::new(bundle)));
                    }
                    Request::Diff { path, token } => {
                        match compute_diff(
                            &git,
                            &path,
                            base_override.as_deref(),
                            config_base.as_deref(),
                            &mut cache,
                        ) {
                            Ok(diff) => {
                                let _ = resp_tx.send(Response::Diff { path, token, diff });
                            }
                            Err(e) => {
                                let _ = resp_tx.send(Response::Error(format!("diff {path}: {e}")));
                            }
                        }
                    }
                    Request::SwitchRepo(new_path) => match GitRepo::new(&new_path) {
                        Ok(new_git) => {
                            git = new_git;
                            cache.clear();
                            let _ = resp_tx.send(Response::SwitchAck(true));
                        }
                        Err(e) => {
                            let _ = resp_tx.send(Response::Error(format!("switch: {e}")));
                            let _ = resp_tx.send(Response::SwitchAck(false));
                        }
                    },
                    Request::Shutdown => return,
                }
            }
        }
    }
}

/// Compute the same status bundle the old synchronous `handle_fs_change`
/// produced. Errors degrade to an empty / default field rather than failing
/// the whole bundle — mirrors current "best-effort" semantics.
///
/// `cache` is consulted / populated only when no explicit override is in
/// play. Explicit overrides always short-circuit and bypass the cache.
fn compute_status(
    git: &GitRepo,
    base_override: Option<&str>,
    config_base: Option<&str>,
    cache: &mut BaseCache,
) -> StatusBundle {
    let current_branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());

    // Priority 1 + 2: explicit overrides short-circuit detection entirely.
    if let Some(explicit) = base_override.or(config_base) {
        tracing::debug!(
            target: "git_rt::git::base_detect",
            branch = %current_branch,
            base = %explicit,
            "explicit override active; skipping detection"
        );
        return compute_with_base(git, Some(explicit.to_string()), current_branch);
    }

    // Priority 3: cache hit.
    let resolved_base: Option<String> = if let Some(cached) = cache.get(&current_branch) {
        tracing::debug!(
            target: "git_rt::git::base_detect",
            branch = %current_branch,
            base = ?cached.as_deref(),
            "cache hit"
        );
        cached.clone()
    } else {
        // Priority 4: detect.
        tracing::debug!(
            target: "git_rt::git::base_detect",
            branch = %current_branch,
            "cache miss, detecting"
        );
        let detected = git
            .detect_base_branch(&current_branch)
            .filter(|s| !s.is_empty());
        // Priority 5: resolve_base_branch fallback if detection gave up
        // OR returned an empty string (defensive — should not happen).
        let final_base = detected.or_else(|| {
            tracing::debug!(
                target: "git_rt::git::base_detect",
                branch = %current_branch,
                "fallback: delegating to resolve_base_branch"
            );
            git.resolve_base_branch(None)
        });
        cache.insert(current_branch.clone(), final_base.clone());
        final_base
    };

    compute_with_base(git, resolved_base, current_branch)
}

fn compute_with_base(
    git: &GitRepo,
    resolved_base: Option<String>,
    current_branch: String,
) -> StatusBundle {
    let (merge_base, files) = match resolved_base.as_deref() {
        Some(base_name) => match git.merge_base(base_name) {
            Ok(Some(mb)) => match git.branch_status(mb) {
                Ok(f) => (Some(mb), f),
                Err(_) => (None, git.status().unwrap_or_default()),
            },
            _ => (None, git.status().unwrap_or_default()),
        },
        None => (None, git.status().unwrap_or_default()),
    };

    StatusBundle {
        files,
        merge_base,
        base_branch: resolved_base.unwrap_or_default(),
        branch: current_branch,
        head: git.head_info().ok(),
        stash_count: git.stash_count().unwrap_or(0),
        ahead_behind: git.ahead_behind().unwrap_or(None),
        repo_state: git.repo_state(),
        repo_name: git.repo_name(),
        worktree_name: git.worktree_name(),
    }
}

/// Resolve the base branch for the current HEAD, mirroring the priority
/// chain in [`compute_status`]:
///
/// 1. explicit CLI override
/// 2. config file override
/// 3. cached detection result for the current branch
/// 4. `detect_base_branch` fresh detection
/// 5. `resolve_base_branch(None)` fallback
///
/// Explicit overrides short-circuit and bypass the cache.
fn resolve_base_with_cache(
    git: &GitRepo,
    base_override: Option<&str>,
    config_base: Option<&str>,
    cache: &mut BaseCache,
) -> Option<String> {
    if let Some(explicit) = base_override.or(config_base) {
        return Some(explicit.to_string());
    }

    let current_branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());

    if let Some(cached) = cache.get(&current_branch) {
        return cached.clone();
    }

    let detected = git
        .detect_base_branch(&current_branch)
        .filter(|s| !s.is_empty());
    let final_base = detected.or_else(|| git.resolve_base_branch(None));
    cache.insert(current_branch, final_base.clone());
    final_base
}

/// Compute a single-file diff. Uses branch diff if a merge base is available,
/// otherwise falls back to working-tree diff. `cache` is consulted for base
/// branch resolution so repeated Diff requests don't re-run detection.
fn compute_diff(
    git: &GitRepo,
    path: &str,
    base_override: Option<&str>,
    config_base: Option<&str>,
    cache: &mut BaseCache,
) -> Result<FileDiff, crate::git::GitFailure> {
    let resolved_base = resolve_base_with_cache(git, base_override, config_base, cache);
    if let Some(base_name) = resolved_base {
        if let Ok(Some(mb)) = git.merge_base(&base_name) {
            return git.branch_diff_file(path, mb);
        }
    }
    git.diff_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(q: VecDeque<Request>) -> Vec<&'static str> {
        q.into_iter()
            .map(|r| match r {
                Request::Recompute => "R",
                Request::Diff { .. } => "D",
                Request::SwitchRepo(_) => "S",
                Request::Shutdown => "X",
            })
            .collect()
    }

    fn diff_req(token: u64) -> Request {
        Request::Diff {
            path: format!("p{}", token),
            token,
        }
    }

    #[test]
    fn coalesce_empty() {
        let out = coalesce(VecDeque::new());
        assert!(out.is_empty());
    }

    #[test]
    fn coalesce_single_recompute_passes_through() {
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        assert_eq!(collect(coalesce(input)), vec!["R"]);
    }

    #[test]
    fn coalesce_three_recomputes_collapse_to_one() {
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        input.push_back(Request::Recompute);
        input.push_back(Request::Recompute);
        assert_eq!(collect(coalesce(input)), vec!["R"]);
    }

    #[test]
    fn coalesce_preserves_shutdown_after_recompute() {
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        input.push_back(Request::Shutdown);
        let out = coalesce(input);
        assert_eq!(collect(out), vec!["X", "R"]);
    }

    #[test]
    fn coalesce_preserves_switchrepo() {
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        input.push_back(Request::SwitchRepo(PathBuf::from("/tmp/repo")));
        input.push_back(Request::Recompute);
        let out = collect(coalesce(input));
        assert!(out.contains(&"S"));
        let r_count = out.iter().filter(|s| **s == "R").count();
        assert_eq!(r_count, 1);
    }

    #[test]
    fn coalesce_preserves_diffs_in_order() {
        let mut q = VecDeque::new();
        q.push_back(diff_req(1));
        q.push_back(diff_req(2));
        q.push_back(diff_req(3));
        let out = coalesce(q);
        assert_eq!(collect(out), vec!["D", "D", "D"]);
    }

    #[test]
    fn coalesce_diff_then_recompute_orders_diff_first() {
        let mut q = VecDeque::new();
        q.push_back(Request::Recompute);
        q.push_back(diff_req(1));
        q.push_back(Request::Recompute);
        let out = coalesce(q);
        // All Diffs preserved; the single Recompute is appended at the end.
        assert_eq!(collect(out), vec!["D", "R"]);
    }

    #[test]
    fn worker_recompute_returns_status() {
        use crossbeam_channel::bounded;
        use std::time::Duration;

        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().to_path_buf();

        // Init a real git repo so GitRepo::new succeeds.
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&repo_path)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "init",
            ])
            .current_dir(&repo_path)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();

        let (req_tx, req_rx) = bounded::<Request>(8);
        let (resp_tx, resp_rx) = bounded::<Response>(8);

        let handle = Worker::spawn(repo_path.clone(), None, None, req_rx, resp_tx);

        req_tx.send(Request::Recompute).unwrap();
        let resp = resp_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker did not respond");

        match resp {
            Response::Status(_) => {} // success
            other => panic!("expected Status, got {:?}", other),
        }

        req_tx.send(Request::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn base_cache_starts_empty() {
        let cache = BaseCache::new();
        assert!(cache.is_empty());
    }

    #[test]
    fn base_cache_insert_and_get() {
        let mut cache = BaseCache::new();
        cache.insert("feature-b".to_string(), Some("feature-a".to_string()));
        assert_eq!(
            cache.get("feature-b"),
            Some(Some("feature-a".to_string())).as_ref()
        );
    }

    #[test]
    fn base_cache_caches_none_result() {
        let mut cache = BaseCache::new();
        cache.insert("orphan".to_string(), None);
        // `None` entry must be distinguishable from "not cached".
        assert!(cache.get("orphan").is_some());
        assert!(cache.get("orphan").unwrap().is_none());
        assert!(cache.get("other").is_none());
    }

    #[test]
    fn base_cache_clear_removes_all_entries() {
        let mut cache = BaseCache::new();
        cache.insert("a".to_string(), Some("main".to_string()));
        cache.insert("b".to_string(), None);
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn compute_status_populates_cache_on_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        let g = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .expect("git must run");
            assert!(status.success(), "git {:?} failed", args);
        };
        g(&["init", "-q", "-b", "main"]);
        g(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ]);
        g(&["checkout", "-q", "-b", "feature-a"]);
        g(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "a1",
        ]);

        let git = GitRepo::new(p).unwrap();
        let mut cache = BaseCache::new();
        let bundle = compute_status(&git, None, None, &mut cache);
        assert_eq!(bundle.base_branch, "main");
        assert_eq!(
            cache.get("feature-a"),
            Some(Some("main".to_string())).as_ref(),
            "detection result should be cached under the current branch"
        );
    }

    #[test]
    fn compute_status_hits_cache_and_skips_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        let g = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .expect("git must run");
            assert!(status.success(), "git {:?} failed", args);
        };
        g(&["init", "-q", "-b", "main"]);
        g(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ]);
        g(&["checkout", "-q", "-b", "feature-a"]);
        g(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "a1",
        ]);

        // Pre-populate the cache with a distinctive fake base that
        // detection would NEVER produce. If compute_status returns this
        // value, we know the cache short-circuited detection.
        let mut cache = BaseCache::new();
        cache.insert(
            "feature-a".to_string(),
            Some("definitely-not-a-real-branch".to_string()),
        );

        let git = GitRepo::new(p).unwrap();
        let bundle = compute_status(&git, None, None, &mut cache);

        assert_eq!(
            bundle.base_branch, "definitely-not-a-real-branch",
            "cache should have short-circuited detection"
        );
    }

    #[test]
    fn compute_status_with_explicit_override_skips_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        let g = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .expect("git must run");
            assert!(status.success(), "git {:?} failed", args);
        };
        g(&["init", "-q", "-b", "main"]);
        g(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ]);
        g(&["checkout", "-q", "-b", "feature-a"]);
        g(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "a1",
        ]);

        let git = GitRepo::new(p).unwrap();
        let mut cache = BaseCache::new();
        let bundle = compute_status(&git, Some("main"), None, &mut cache);
        assert_eq!(bundle.base_branch, "main");
        assert!(
            cache.is_empty(),
            "explicit override must not populate the cache"
        );
    }

    #[test]
    fn switch_repo_clears_base_cache() {
        use crossbeam_channel::bounded;
        use std::time::Duration;

        // Build two tiny repos.
        let tmp_a = tempfile::tempdir().unwrap();
        let tmp_b = tempfile::tempdir().unwrap();
        for dir in [tmp_a.path(), tmp_b.path()] {
            let init_status = std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(dir)
                .status()
                .expect("git init must run");
            assert!(init_status.success(), "git init failed in {:?}", dir);
            let commit_status = std::process::Command::new("git")
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--allow-empty",
                    "-q",
                    "-m",
                    "init",
                ])
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .expect("git commit must run");
            assert!(commit_status.success(), "git commit failed in {:?}", dir);
        }

        let (req_tx, req_rx) = bounded::<Request>(8);
        let (resp_tx, resp_rx) = bounded::<Response>(8);
        let handle = Worker::spawn(tmp_a.path().to_path_buf(), None, None, req_rx, resp_tx);

        // First recompute populates cache for repo A.
        req_tx.send(Request::Recompute).unwrap();
        let _ = resp_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // Switch to repo B — cache must be cleared so repo A's answer
        // doesn't leak.
        req_tx
            .send(Request::SwitchRepo(tmp_b.path().to_path_buf()))
            .unwrap();
        let ack = resp_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match ack {
            Response::SwitchAck(true) => {}
            other => panic!("expected SwitchAck(true), got {:?}", other),
        }

        // Recompute against B — must succeed without stale state.
        req_tx.send(Request::Recompute).unwrap();
        let resp = resp_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match resp {
            Response::Status(b) => {
                // Repo B has no remote, so no base — but the key assertion
                // is that no panic / stale data bled through.
                assert_eq!(b.branch, "main");
            }
            other => panic!("expected Status, got {:?}", other),
        }

        req_tx.send(Request::Shutdown).unwrap();
        handle.join().unwrap();
    }
}
