//! Shell-out to the `git` CLI for fast status enumeration.
//!
//! On large repos (300k+ files), `gix::status().into_index_worktree_iter()`
//! takes minutes. `git status --porcelain=v2` does the same job in seconds
//! because git has 20 years of optimization (untracked cache, fsmonitor,
//! parallel walk). This module wraps the CLI call + numstat output and
//! merges them into the existing `FileEntry` shape.

#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::process::Command;

#[allow(unused_imports)]
use crate::git::{FileEntry, FileStatus, GitFailure};

/// Parse `git status --porcelain=v2 -z` output into a list of
/// `(path, FileStatus)` pairs. Robust to records appearing in any order;
/// callers downstream sort by path.
///
/// Handles entry types:
///   `1 XY ...`  — ordinary changed entries
///   `2 XY ...`  — renames/copies (emitted as Deleted(from) + Added(to))
///   `u XY ...`  — unmerged (Conflicted)
///   `? <path>`  — untracked
///   `! <path>`  — ignored (filtered — we never pass `--ignored`)
///
/// `-z` mode: records are NUL-terminated; for type-2 entries the original
/// path follows in the next NUL-terminated chunk.
pub fn parse_porcelain_v2(bytes: &[u8]) -> Vec<(String, FileStatus)> {
    let mut out = Vec::new();
    let mut chunks = bytes.split(|&b| b == 0).filter(|c| !c.is_empty());
    while let Some(chunk) = chunks.next() {
        match chunk.first() {
            Some(&b'?') => {
                // "? <path>" — strip the 2-byte prefix
                if let Some(path) = chunk.get(2..) {
                    out.push((
                        String::from_utf8_lossy(path).into_owned(),
                        FileStatus::Untracked,
                    ));
                }
            }
            _ => {
                // Other entry types implemented in following tasks.
                let _ = &mut chunks; // silence unused-mut once more arms exist
            }
        }
    }
    out
}
