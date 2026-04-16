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
            Some(&b'2') => {
                // Rename: parse new path from this chunk, original path from next chunk.
                if let Some(new_path) = parse_type2_new_path(chunk) {
                    if let Some(orig_chunk) = chunks.next() {
                        let orig = String::from_utf8_lossy(orig_chunk).into_owned();
                        out.push((orig, FileStatus::Deleted));
                        out.push((new_path, FileStatus::Added));
                    }
                }
            }
            Some(&b'u') => {
                if let Some(path) = parse_type_u(chunk) {
                    out.push((path, FileStatus::Conflicted));
                }
            }
            _ => {
                // `! ignored` lines and unknown markers — skip
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

/// Parse type-2 entry, returning the new (destination) path.
/// Format: `2 XY sub mH mI mW hH hI Xscore <newPath>`.
/// The original path follows in the next NUL-terminated chunk.
fn parse_type2_new_path(chunk: &[u8]) -> Option<String> {
    // 0=marker, 1=XY, 2=sub, 3=mH, 4=mI, 5=mW, 6=hH, 7=hI, 8=Xscore, 9..=newPath
    let mut parts = chunk.splitn(10, |&b| b == b' ');
    for _ in 0..9 {
        parts.next()?;
    }
    let path = parts.next()?;
    Some(String::from_utf8_lossy(path).into_owned())
}

/// Parse type-u (unmerged) entry, returning the path.
/// Format: `u XY sub m1 m2 m3 mW h1 h2 h3 <path>`.
fn parse_type_u(chunk: &[u8]) -> Option<String> {
    // 0=marker, 1=XY, 2=sub, 3=m1, 4=m2, 5=m3, 6=mW, 7=h1, 8=h2, 9=h3, 10..=path
    let mut parts = chunk.splitn(11, |&b| b == b' ');
    for _ in 0..10 {
        parts.next()?;
    }
    let path = parts.next()?;
    Some(String::from_utf8_lossy(path).into_owned())
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

/// Parse `git diff --numstat -z <ref>` output into `(path, insertions, deletions)`.
///
/// **Trust assumption:** like [`parse_porcelain_v2`], this consumes the output
/// of a stable git CLI command and does not validate against malformed inputs
/// that git cannot emit.
///
/// Format:
///   Regular:  `<added>\t<deleted>\t<path>\0`
///   Binary:   `-\t-\t<path>\0`     → reported as (path, 0, 0)
///   Rename:   `<added>\t<deleted>\t\0<from>\0<to>\0`
///             → emitted under the destination path; the source name is dropped
///             (status output already reports the deletion via porcelain).
pub fn parse_numstat(bytes: &[u8]) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut chunks = bytes.split(|&b| b == 0).filter(|c| !c.is_empty());
    while let Some(chunk) = chunks.next() {
        // Each chunk is "<added>\t<deleted>\t<path-or-empty>".
        let mut fields = chunk.splitn(3, |&b| b == b'\t');
        let added_b = match fields.next() {
            Some(f) => f,
            None => continue,
        };
        let deleted_b = match fields.next() {
            Some(f) => f,
            None => continue,
        };
        let path_b = match fields.next() {
            Some(f) => f,
            None => continue,
        };

        let added = parse_count(added_b);
        let deleted = parse_count(deleted_b);

        if path_b.is_empty() {
            // Rename: drop the next chunk (source path), use the one after as destination.
            let _from = chunks.next();
            let to = match chunks.next() {
                Some(c) => c,
                None => continue,
            };
            out.push((String::from_utf8_lossy(to).into_owned(), added, deleted));
        } else {
            out.push((String::from_utf8_lossy(path_b).into_owned(), added, deleted));
        }
    }
    out
}

/// Parse a numstat count field. `-` (binary file marker) → 0.
fn parse_count(bytes: &[u8]) -> usize {
    if bytes == b"-" {
        return 0;
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
}

/// Merge status and numstat outputs into a sorted `Vec<FileEntry>`.
///
/// `repo_root` is used to resolve untracked-file paths so we can read the
/// file from disk and count lines (treated as insertions, matching the
/// pre-existing `branch_status` behavior).
///
/// Output is sorted by `path` ascending — matches the contract of the old
/// `branch_status` / `status` implementations.
pub fn merge_status_and_numstat(
    status: Vec<(String, FileStatus)>,
    numstat: Vec<(String, usize, usize)>,
    repo_root: &Path,
) -> Vec<FileEntry> {
    use std::collections::HashMap;

    // Index numstat by path for O(1) lookup.
    let mut numstat_map: HashMap<String, (usize, usize)> = numstat
        .into_iter()
        .map(|(p, ins, del)| (p, (ins, del)))
        .collect();

    let mut entries: Vec<FileEntry> = status
        .into_iter()
        .map(|(path, fs_status)| {
            let (insertions, deletions) = if let Some((ins, del)) = numstat_map.remove(&path) {
                (ins, del)
            } else if matches!(fs_status, FileStatus::Untracked) {
                // Untracked files have no numstat entry — read from disk to
                // count lines (synthetic "all-additions" diff).
                let abs = repo_root.join(&path);
                let lines = std::fs::read_to_string(&abs)
                    .map(|s| s.lines().count())
                    .unwrap_or(0);
                (lines, 0)
            } else {
                (0, 0)
            };

            FileEntry {
                path,
                status: fs_status,
                insertions,
                deletions,
            }
        })
        .collect();

    // Defensive: if numstat reported something with no matching status entry
    // (race condition between the two `git` calls), surface it as Modified
    // so it doesn't silently disappear.
    for (path, (insertions, deletions)) in numstat_map {
        entries.push(FileEntry {
            path,
            status: FileStatus::Modified,
            insertions,
            deletions,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}
