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
    Status(StatusBundle),
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
                        let _ = resp_tx.send(Response::Status(bundle));
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

/// Compute the same status bundle the old synchronous `handle_fs_change`
/// produced. Errors degrade to an empty / default field rather than failing
/// the whole bundle — mirrors current "best-effort" semantics.
fn compute_status(
    git: &GitRepo,
    base_override: Option<&str>,
    config_base: Option<&str>,
) -> StatusBundle {
    let resolved_base = git.resolve_base_branch(base_override.or(config_base));
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
        branch: git.branch_name().unwrap_or_else(|_| "HEAD".to_string()),
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
    fn coalesce_preserves_diff_and_shutdown_in_order() {
        let mut input = VecDeque::new();
        input.push_back(diff_req(1));
        input.push_back(Request::Shutdown);
        let out = coalesce(input);
        assert_eq!(collect(out), vec!["D", "X"]);
    }

    #[test]
    fn coalesce_keeps_diff_when_recompute_collapses() {
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        input.push_back(diff_req(1));
        input.push_back(Request::Recompute);
        input.push_back(diff_req(2));
        input.push_back(Request::Recompute);
        // Diffs preserved in order; the one Recompute lands at the end.
        assert_eq!(collect(coalesce(input)), vec!["D", "D", "R"]);
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
    fn coalesce_recompute_then_diff_orders_diff_first() {
        // Bug repro: [R, R, D] must produce [D, R] (Diff first, then the one Recompute).
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        input.push_back(Request::Recompute);
        input.push_back(diff_req(1));
        assert_eq!(collect(coalesce(input)), vec!["D", "R"]);
    }

    #[test]
    fn coalesce_three_recomputes_then_two_diffs() {
        // [R, R, R, D, D] must produce [D, D, R].
        let mut input = VecDeque::new();
        input.push_back(Request::Recompute);
        input.push_back(Request::Recompute);
        input.push_back(Request::Recompute);
        input.push_back(diff_req(1));
        input.push_back(diff_req(2));
        assert_eq!(collect(coalesce(input)), vec!["D", "D", "R"]);
    }

    #[test]
    fn coalesce_diff_recompute_recompute_switchrepo() {
        // Heterogeneous batch: [D, R, R, S] -> [D, S, R].
        let mut input = VecDeque::new();
        input.push_back(diff_req(1));
        input.push_back(Request::Recompute);
        input.push_back(Request::Recompute);
        input.push_back(Request::SwitchRepo(PathBuf::from("/tmp/repo")));
        assert_eq!(collect(coalesce(input)), vec!["D", "S", "R"]);
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
            .args(["commit", "--allow-empty", "-q", "-m", "init"])
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
}
