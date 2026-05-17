//! Shell-out to the `git` CLI for fast status enumeration.
//!
//! On large repos (300k+ files), `gix::status().into_index_worktree_iter()`
//! takes minutes. `git status --porcelain=v2` does the same job in seconds
//! because git has 20 years of optimization (untracked cache, fsmonitor,
//! parallel walk). This module wraps the CLI call + numstat output and
//! merges them into the existing `FileEntry` shape.

use std::path::Path;
use std::process::Command;

use crate::git::{
    ChangeGroup, DiffHunk, DiffLine, DiffLineKind, FileDiff, FileEntry, FileStatus, GitFailure,
};

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
#[tracing::instrument(name = "git.parse_porcelain_v2", skip_all, fields(bytes = bytes.len()))]
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
#[tracing::instrument(name = "git.parse_numstat", skip_all, fields(bytes = bytes.len()))]
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

/// Classify a file present in `git status` into a `ChangeGroup`.
///
/// A file present in `git status` belongs to `New` if untracked, else
/// `Changes`. Files absent from `git status` (numstat-only) are `Committed`
/// and assigned by the caller.
fn group_for_status(status: &FileStatus) -> ChangeGroup {
    match status {
        FileStatus::Untracked => ChangeGroup::New,
        _ => ChangeGroup::Changes,
    }
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
    name_status: Option<std::collections::HashMap<String, FileStatus>>,
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
                group: group_for_status(&fs_status),
                status: fs_status,
                insertions,
                deletions,
            }
        })
        .collect();

    // Files in numstat but not in `git status` are committed changes that are
    // clean in the working tree (e.g. a file added or modified on the branch
    // but not touched since the commit). Use name-status when available for
    // accurate classification; fall back to Modified as a safe default.
    for (path, (insertions, deletions)) in numstat_map {
        let status = name_status
            .as_ref()
            .and_then(|ns| ns.get(&path).cloned())
            .unwrap_or(FileStatus::Modified);
        entries.push(FileEntry {
            path,
            status,
            insertions,
            deletions,
            group: ChangeGroup::Committed,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// Run `git status` + `git diff --numstat` against `repo_path` and merge
/// the results into a sorted `Vec<FileEntry>`.
///
/// `base_ref`:
///   - `Some(oid)` → branch view: numstat is `git diff --numstat -z <oid>`,
///     which gives the merge-base-to-worktree union (committed
///     on this branch + uncommitted) in one shot.
///   - `None` → non-branch view: numstat is `git diff --numstat -z`,
///     which gives HEAD-to-worktree (staged + unstaged).
///
/// Both invocations include `-c core.quotePath=false` so non-ASCII paths
/// come through unquoted and parse cleanly under `-z`.
///
/// Errors map to [`GitFailure::EnvChange`] so the worker can log and the
/// app can hold its previous state during transient git env changes.
#[tracing::instrument(name = "git.compute_status_files", skip_all)]
pub fn compute_status_files(
    repo_path: &Path,
    base_ref: Option<&gix::ObjectId>,
) -> Result<Vec<FileEntry>, GitFailure> {
    use std::collections::HashMap;
    let status_bytes = run_status(repo_path)?;
    let numstat_bytes = run_numstat(repo_path, base_ref)?;
    let status = parse_porcelain_v2(&status_bytes);
    let numstat = parse_numstat(&numstat_bytes);
    // When diffing against a merge base, also fetch `--name-status` so we can
    // accurately classify committed-only changes that don't appear in
    // `git status` (e.g. a file added or modified on the branch but left clean
    // in the worktree since the commit).
    let name_status: Option<HashMap<String, FileStatus>> = if base_ref.is_some() {
        let ns_bytes = run_diff_name_status(repo_path, base_ref)?;
        Some(parse_name_status(&ns_bytes).into_iter().collect())
    } else {
        None
    };
    Ok(merge_status_and_numstat(
        status,
        numstat,
        repo_path,
        name_status,
    ))
}

fn run_status(repo_path: &Path) -> Result<Vec<u8>, GitFailure> {
    let out = Command::new("git")
        .current_dir(repo_path)
        .args([
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=normal",
        ])
        .output()
        .map_err(|e| GitFailure::EnvChange(format!("git status spawn: {e}")))?;
    if !out.status.success() {
        return Err(GitFailure::EnvChange(format!(
            "git status exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out.stdout)
}

fn run_numstat(repo_path: &Path, base_ref: Option<&gix::ObjectId>) -> Result<Vec<u8>, GitFailure> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .args(["-c", "core.quotePath=false", "diff", "--numstat", "-z"]);
    if let Some(oid) = base_ref {
        cmd.arg(format!("{}", oid.to_hex()));
    }
    let out = cmd
        .output()
        .map_err(|e| GitFailure::EnvChange(format!("git diff spawn: {e}")))?;
    if !out.status.success() {
        return Err(GitFailure::EnvChange(format!(
            "git diff exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out.stdout)
}

/// Run `git diff --name-status -z <base_ref>` and return raw stdout.
fn run_diff_name_status(
    repo_path: &Path,
    base_ref: Option<&gix::ObjectId>,
) -> Result<Vec<u8>, GitFailure> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .args(["-c", "core.quotePath=false", "diff", "--name-status", "-z"]);
    if let Some(oid) = base_ref {
        cmd.arg(format!("{}", oid.to_hex()));
    }
    let out = cmd
        .output()
        .map_err(|e| GitFailure::EnvChange(format!("git diff --name-status spawn: {e}")))?;
    if !out.status.success() {
        return Err(GitFailure::EnvChange(format!(
            "git diff --name-status exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out.stdout)
}

/// Parse `git diff --name-status -z` output into `(path, FileStatus)` pairs.
///
/// Format: each record is `<status>\0<path>\0` for ordinary entries, or
/// `R<score>\0<old-path>\0<new-path>\0` for renames.
///
/// Status letters: A=Added, M=Modified, D=Deleted, R=Renamed, C=Copied,
/// T=Type-change, U=Unmerged, X=Unknown. We map to our `FileStatus` enum.
pub fn parse_name_status(bytes: &[u8]) -> Vec<(String, FileStatus)> {
    let mut out = Vec::new();
    let mut chunks = bytes.split(|&b| b == 0).filter(|c| !c.is_empty());
    while let Some(status_chunk) = chunks.next() {
        let status_char = status_chunk.first().copied().unwrap_or(b'?');
        let fs = match status_char {
            b'A' => FileStatus::Added,
            b'D' => FileStatus::Deleted,
            b'R' | b'C' => {
                // Rename/copy: skip old path, use new path.
                let _old = chunks.next();
                if let Some(new_path) = chunks.next() {
                    out.push((
                        String::from_utf8_lossy(new_path).into_owned(),
                        FileStatus::Renamed,
                    ));
                }
                continue;
            }
            _ => FileStatus::Modified,
        };
        if let Some(path_chunk) = chunks.next() {
            out.push((String::from_utf8_lossy(path_chunk).into_owned(), fs));
        }
    }
    out
}

/// Parse the unified-diff body of a single-file `git diff -p` invocation
/// into a [`FileDiff`].
///
/// Recognised line kinds:
///   `@@ ... @@ ...`  — hunk header (starts a new hunk)
///   ` `              — context line
///   `-`              — deletion
///   `+`              — addition
///   `\`              — "no newline at end of file" marker (skipped)
///
/// All header lines (`diff --git`, `index`, `---`, `+++`, `Binary files ...`)
/// before the first `@@` are skipped. Binary diffs and identical files
/// produce an empty [`FileDiff`].
pub fn parse_unified_diff(bytes: &[u8]) -> FileDiff {
    let text = String::from_utf8_lossy(bytes);
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current: Option<DiffHunk> = None;

    for line in text.split('\n') {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            continue;
        };

        if line.starts_with('\\') {
            continue;
        }

        let (kind, content) = match line.as_bytes().first() {
            Some(b'+') => (DiffLineKind::Addition, &line[1..]),
            Some(b'-') => (DiffLineKind::Deletion, &line[1..]),
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            None => continue,
            _ => continue,
        };

        hunk.lines.push(DiffLine {
            kind,
            content: content.to_string(),
        });
    }

    if let Some(h) = current.take() {
        hunks.push(h);
    }

    FileDiff { hunks }
}

/// Run `git diff -p --no-color [base] -- <path>` and return raw stdout.
///
/// `base_ref`:
///   - `Some(oid)` → diff merge-base-tree to worktree (branch view)
///   - `None`      → diff index to worktree (default working-tree view,
///     i.e. unstaged changes only — matches the prior in-tree behaviour)
///
/// Includes `-c core.quotePath=false` so non-ASCII paths come through unquoted.
/// Note: we deliberately do NOT pass `-z` here because diff bodies contain
/// newlines, and `-z` only affects the inter-record path NULs which we don't
/// need for a single-file invocation.
pub fn run_diff_patch(
    repo_path: &Path,
    base_ref: Option<&gix::ObjectId>,
    path: &str,
) -> Result<Vec<u8>, GitFailure> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .args(["-c", "core.quotePath=false", "diff", "-p", "--no-color"]);
    if let Some(oid) = base_ref {
        cmd.arg(format!("{}", oid.to_hex()));
    }
    cmd.arg("--").arg(path);
    let out = cmd
        .output()
        .map_err(|e| GitFailure::EnvChange(format!("git diff -p spawn: {e}")))?;
    if !out.status.success() {
        return Err(GitFailure::EnvChange(format!(
            "git diff -p exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod diff_parser_tests {
    use super::*;

    fn join_patch(lines: &[&str]) -> Vec<u8> {
        let mut s = lines.join("\n");
        s.push('\n');
        s.into_bytes()
    }

    #[test]
    fn parses_single_hunk() {
        let patch = join_patch(&[
            "diff --git a/foo b/foo",
            "index 1111..2222 100644",
            "--- a/foo",
            "+++ b/foo",
            "@@ -1,3 +1,3 @@",
            " a",
            "-b",
            "+B",
            " c",
        ]);
        let diff = parse_unified_diff(&patch);
        assert_eq!(diff.hunks.len(), 1);
        let h = &diff.hunks[0];
        assert_eq!(h.header, "@@ -1,3 +1,3 @@");
        assert_eq!(h.lines.len(), 4);
        assert!(matches!(h.lines[0].kind, DiffLineKind::Context));
        assert_eq!(h.lines[0].content, "a");
        assert!(matches!(h.lines[1].kind, DiffLineKind::Deletion));
        assert_eq!(h.lines[1].content, "b");
        assert!(matches!(h.lines[2].kind, DiffLineKind::Addition));
        assert_eq!(h.lines[2].content, "B");
        assert!(matches!(h.lines[3].kind, DiffLineKind::Context));
        assert_eq!(h.lines[3].content, "c");
    }

    #[test]
    fn parses_multiple_non_contiguous_hunks() {
        // This is the bug we're fixing: a +10/-1 file with changes at the
        // top and bottom should produce TWO hunks, not one giant
        // delete-then-add block.
        let patch = join_patch(&[
            "diff --git a/foo b/foo",
            "--- a/foo",
            "+++ b/foo",
            "@@ -1,3 +1,3 @@",
            "-old line 1",
            "+new line 1",
            " unchanged 2",
            " unchanged 3",
            "@@ -50,2 +50,3 @@",
            " ctx",
            "+inserted",
            " ctx",
        ]);
        let diff = parse_unified_diff(&patch);
        assert_eq!(diff.hunks.len(), 2, "expected two separate hunks");
        assert_eq!(diff.hunks[0].header, "@@ -1,3 +1,3 @@");
        assert_eq!(diff.hunks[1].header, "@@ -50,2 +50,3 @@");

        let total_dels = diff
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| matches!(l.kind, DiffLineKind::Deletion))
            .count();
        let total_adds = diff
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| matches!(l.kind, DiffLineKind::Addition))
            .count();
        assert_eq!(total_dels, 1);
        assert_eq!(total_adds, 2);
    }

    #[test]
    fn skips_no_newline_marker() {
        let patch = join_patch(&[
            "--- a/foo",
            "+++ b/foo",
            "@@ -1 +1 @@",
            "-x",
            "\\ No newline at end of file",
            "+y",
            "\\ No newline at end of file",
        ]);
        let diff = parse_unified_diff(&patch);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].lines.len(), 2);
    }

    #[test]
    fn empty_output_yields_empty_diff() {
        let diff = parse_unified_diff(b"");
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn binary_files_yield_empty_diff() {
        // `git diff` emits "Binary files a/foo and b/foo differ" with no @@.
        let patch = join_patch(&[
            "diff --git a/foo b/foo",
            "index 1111..2222 100644",
            "Binary files a/foo and b/foo differ",
        ]);
        let diff = parse_unified_diff(&patch);
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn header_lines_before_first_hunk_are_ignored() {
        // The leading `+++` / `---` lines must not be parsed as additions/deletions.
        let patch = join_patch(&[
            "diff --git a/foo b/foo",
            "--- a/foo",
            "+++ b/foo",
            "@@ -1 +1 @@",
            "-a",
            "+b",
        ]);
        let diff = parse_unified_diff(&patch);
        assert_eq!(diff.hunks.len(), 1);
        let lines = &diff.hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert!(matches!(lines[0].kind, DiffLineKind::Deletion));
        assert_eq!(lines[0].content, "a");
        assert!(matches!(lines[1].kind, DiffLineKind::Addition));
        assert_eq!(lines[1].content, "b");
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::git::ChangeGroup;

    #[test]
    fn classifies_files_into_change_groups() {
        let tmp = std::env::temp_dir();
        let status = vec![
            ("changed.rs".to_string(), FileStatus::Modified),
            ("staged.rs".to_string(), FileStatus::Staged),
            ("brand_new.rs".to_string(), FileStatus::Untracked),
        ];
        let numstat = vec![
            ("changed.rs".to_string(), 1, 1),
            ("staged.rs".to_string(), 2, 0),
            ("committed.rs".to_string(), 5, 5),
        ];
        let entries = merge_status_and_numstat(status, numstat, &tmp, None);

        let group = |path: &str| entries.iter().find(|e| e.path == path).unwrap().group;
        assert_eq!(group("changed.rs"), ChangeGroup::Changes);
        assert_eq!(group("staged.rs"), ChangeGroup::Changes);
        assert_eq!(group("brand_new.rs"), ChangeGroup::New);
        assert_eq!(group("committed.rs"), ChangeGroup::Committed);
    }
}
