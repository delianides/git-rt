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
/// **Trust assumption:** this parser consumes the output of
/// `git status --porcelain=v2 -z`, a stable spec'd format. Defensive
/// validation for inputs git cannot emit (malformed XY codes, no-change
/// entries, etc.) is intentionally omitted — invalid inputs would indicate
/// git itself is broken, not a bug in the caller.
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
            Some(&b'1') => {
                if let Some((xy, path)) = parse_type1(chunk) {
                    out.push((path, map_xy(xy)));
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

/// Parse `1 XY sub mH mI mW hH hI <path>` → (XY-bytes, path).
/// Returns None if the line doesn't have enough fields.
fn parse_type1(chunk: &[u8]) -> Option<([u8; 2], String)> {
    // Field positions (0-indexed by space-split):
    //   0=marker(1), 1=XY, 2=sub, 3=mH, 4=mI, 5=mW, 6=hH, 7=hI, 8..=path
    // Path may itself contain spaces, so use splitn to keep the tail intact.
    let mut parts = chunk.splitn(9, |&b| b == b' ');
    let _marker = parts.next()?; // "1"
    let xy_bytes = parts.next()?; // "XY"
    if xy_bytes.len() != 2 {
        return None;
    }
    let xy = [xy_bytes[0], xy_bytes[1]];
    // Skip the next 6 metadata fields.
    for _ in 0..6 {
        parts.next()?;
    }
    let path = parts.next()?;
    Some((xy, String::from_utf8_lossy(path).into_owned()))
}

/// Map the porcelain v2 XY status code to our `FileStatus` enum.
///
/// Priority order (most specific wins):
///   1. Either side is `D` → Deleted
///   2. Worktree side is `M` → Modified (covers `.M` and `MM`)
///   3. Index side is `A` → Added
///   4. Index side is `M` → Staged (covers `M.`)
///   5. Default → Modified (catch-all for unusual combinations)
fn map_xy(xy: [u8; 2]) -> FileStatus {
    let (x, y) = (xy[0], xy[1]);
    if y == b'D' || x == b'D' {
        return FileStatus::Deleted;
    }
    if y == b'M' {
        return FileStatus::Modified;
    }
    if x == b'A' {
        return FileStatus::Added;
    }
    if x == b'M' {
        return FileStatus::Staged;
    }
    FileStatus::Modified
}
