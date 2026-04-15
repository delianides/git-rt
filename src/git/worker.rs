//! Async git worker thread.
//!
//! Owns the [`GitRepo`] and processes git requests on a dedicated thread,
//! keeping the UI event loop responsive regardless of how slow individual
//! git operations are. Communication is via two crossbeam channels:
//! the main thread sends [`Request`] messages and receives [`Response`]
//! messages.

use std::collections::VecDeque;
use std::path::PathBuf;

use crate::git::{FileDiff, FileEntry};

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
}

/// Coalesce a batch of pending requests so redundant `Recompute` messages
/// collapse to a single one. `Diff`, `SwitchRepo`, and `Shutdown` are
/// preserved in FIFO order. Pure function — no I/O, no thread access.
///
/// Behavior:
/// - Multiple `Recompute` → one `Recompute` at the position of the LAST one.
/// - All other variants → preserved in original order.
/// - Empty input → empty output.
pub fn coalesce(input: VecDeque<Request>) -> VecDeque<Request> {
    let mut last_recompute_idx: Option<usize> = None;
    let mut output: VecDeque<Request> = VecDeque::with_capacity(input.len());

    for req in input {
        if matches!(req, Request::Recompute) {
            last_recompute_idx = Some(output.len());
        } else {
            output.push_back(req);
        }
    }

    if let Some(idx) = last_recompute_idx {
        let pos = idx.min(output.len());
        output.insert(pos, Request::Recompute);
    }

    output
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
        let out = collect(coalesce(input));
        let r_count = out.iter().filter(|s| **s == "R").count();
        let d_count = out.iter().filter(|s| **s == "D").count();
        assert_eq!(r_count, 1, "exactly one Recompute should remain: {:?}", out);
        assert_eq!(d_count, 2, "both Diffs should remain: {:?}", out);
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
}
