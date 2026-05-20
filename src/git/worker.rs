//! Async git worker thread.
//!
//! Owns the [`GitRepo`] and processes git requests on a dedicated thread,
//! keeping the UI event loop responsive regardless of how slow individual
//! git operations are. Communication is via two crossbeam channels:
//! the main thread sends [`Request`] messages and receives [`Response`]
//! messages.

use std::collections::VecDeque;
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

/// Returns `true` when a bounded channel has reached or exceeded 50% capacity.
/// `cap == 0` represents an unbounded channel, which is always silent.
// Called by `warn_if_high` below; lib-target dead-code analysis doesn't see
// the binary callers in app.rs, so suppress the lint here.
#[allow(dead_code)]
fn channel_high(len: usize, cap: usize) -> bool {
    cap > 0 && len * 2 >= cap
}

/// Emit a structured `warn!` when a bounded channel is at or above 50%
/// capacity. Call this immediately before sending to `tx` so the log line
/// appears at the moment of pressure, not after the send blocks or drops.
///
/// No-op for unbounded channels (`capacity()` returns `None`).
// Callers live in app.rs (binary target); the lib-target dead-code lint
// doesn't see them, so the allow is required here.
#[allow(dead_code)]
pub(crate) fn warn_if_high(tx: &crossbeam_channel::Sender<Request>, label: &str) {
    let len = tx.len();
    let cap = tx.capacity().unwrap_or(0);
    if channel_high(len, cap) {
        tracing::warn!(channel = label, len, cap, "channel high water mark");
    }
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

    #[tracing::instrument(name = "git.worker.run", skip_all)]
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
                        let bundle =
                            compute_status(&git, base_override.as_deref(), config_base.as_deref());
                        let _ = resp_tx.send(Response::Status(Box::new(bundle)));
                    }
                    Request::Diff { path, token } => {
                        match compute_diff(
                            &git,
                            &path,
                            base_override.as_deref(),
                            config_base.as_deref(),
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

/// Compute the status bundle for the current worktree. Resolves the diff
/// base via strict default-branch resolution and delegates to `compute_with_base`.
/// Errors degrade to default fields rather than failing the whole bundle.
fn compute_status(
    git: &GitRepo,
    base_override: Option<&str>,
    config_base: Option<&str>,
) -> StatusBundle {
    let current_branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());
    let resolved_base = git.resolve_base_branch(base_override.or(config_base));
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

/// Compute a single-file diff. Uses branch diff if a merge base is available,
/// otherwise falls back to working-tree diff.
fn compute_diff(
    git: &GitRepo,
    path: &str,
    base_override: Option<&str>,
    config_base: Option<&str>,
) -> Result<FileDiff, crate::git::GitFailure> {
    let resolved_base = git.resolve_base_branch(base_override.or(config_base));
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
    fn switch_repo_survives_repo_switch() {
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

        // First recompute for repo A.
        req_tx.send(Request::Recompute).unwrap();
        let _ = resp_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // Switch to repo B — worker must swap git handle cleanly.
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

    #[test]
    fn channel_high_predicate() {
        // Below 50%: silent.
        assert!(!channel_high(0, 8));
        assert!(!channel_high(3, 8));
        // At exactly 50%: warn.
        assert!(channel_high(4, 8));
        // Above 50%: warn.
        assert!(channel_high(7, 8));
        // Unbounded (cap == 0): always silent.
        assert!(!channel_high(100, 0));
    }

    #[test]
    fn warn_if_high_smoke() {
        // A bounded sender with some messages queued; just ensure no panic.
        let (tx, _rx) = crossbeam_channel::bounded::<Request>(4);
        tx.send(Request::Recompute).unwrap();
        tx.send(Request::Recompute).unwrap();
        warn_if_high(&tx, "test");
    }

    #[test]
    fn compute_status_uses_origin_head_when_no_override() {
        // Build a real repo with origin/HEAD pointing at main, and a sibling
        // local branch. Assert that compute_status returns base_branch == "main"
        // even from the sibling branch's perspective.
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let upstream = tmp.path().join("upstream.git");
        let work = tmp.path().join("work");

        // Bare upstream
        Command::new("git")
            .args(["init", "-q", "--bare", upstream.to_str().unwrap()])
            .status()
            .unwrap();

        // Working clone with main + sibling branch
        std::fs::create_dir_all(&work).unwrap();
        let g = |args: &[&str]| {
            let s = Command::new("git")
                .args(args)
                .current_dir(&work)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
                .env("GIT_CONFIG_VALUE_0", "false")
                .status()
                .unwrap();
            assert!(s.success(), "git {args:?}");
        };
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(work.join("a"), "x").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "m1"]);
        g(&["remote", "add", "origin", upstream.to_str().unwrap()]);
        g(&["push", "-q", "-u", "origin", "main"]);
        // Set origin/HEAD explicitly
        g(&["remote", "set-head", "origin", "main"]);
        // Sibling branch with a divergent commit
        g(&["checkout", "-q", "-b", "drew/sibling"]);
        std::fs::write(work.join("b"), "y").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "s1"]);
        // Stay on the sibling and run compute_status
        let git = crate::git::GitRepo::new(&work).unwrap();
        let bundle = compute_status(&git, None, None);
        assert_eq!(
            bundle.base_branch, "main",
            "expected strict default-branch base, got {:?}",
            bundle.base_branch
        );
    }

    #[test]
    fn compute_status_falls_back_to_plain_status_without_base() {
        use std::process::Command;

        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();

        let g = |args: &[&str]| {
            let s = Command::new("git")
                .args(args)
                .current_dir(&work)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
                .env("GIT_CONFIG_VALUE_0", "false")
                .status()
                .unwrap();
            assert!(s.success(), "git {args:?}");
        };

        g(&["init", "-q", "-b", "main"]);
        std::fs::write(work.join("tracked.txt"), "one\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "base"]);

        std::fs::write(work.join("committed-only.txt"), "branch\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "branch-only"]);

        std::fs::write(work.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(work.join("untracked.txt"), "new\n").unwrap();

        let git = crate::git::GitRepo::new(&work).unwrap();
        let bundle = compute_status(&git, None, None);

        assert_eq!(bundle.base_branch, "");
        assert!(bundle.merge_base.is_none());
        assert!(
            bundle.files.iter().any(|entry| entry.path == "tracked.txt"),
            "tracked working-tree change should be present: {:?}",
            bundle.files
        );
        assert!(
            bundle
                .files
                .iter()
                .any(|entry| entry.path == "untracked.txt"),
            "untracked file should be present: {:?}",
            bundle.files
        );
        assert!(
            bundle
                .files
                .iter()
                .all(|entry| entry.path != "committed-only.txt"),
            "committed-only branch change should be absent without a base: {:?}",
            bundle.files
        );
    }
}
