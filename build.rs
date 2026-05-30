//! Build script that captures git describe information for the version string.
//!
//! Emits the `PERCH_GIT_DESCRIBE` env var consumed by `src/main.rs`:
//!   - `<7-char-sha>` on a clean tree
//!   - `<7-char-sha>.dirty` when the index or working tree has uncommitted changes
//!   - `unknown` when git is unavailable (e.g. building from a source tarball)
//!
//! # Staleness caveat
//! Cargo re-runs this script only when the listed `rerun-if-changed` paths
//! change.  We watch `.git/HEAD` (catches commits and branch switches) and
//! `.git/index` (catches staged changes, which also cover most `git add`
//! operations).  A pure working-tree edit that has *not* been staged will not
//! trigger a rebuild, so the `.dirty` flag may lag until the next `git add` or
//! any other event that touches `.git/index`.

use std::process::Command;

fn main() {
    // Tell Cargo to re-run this script when HEAD changes (commits, checkouts)
    // or when the index changes (staging). Cargo tolerates missing paths by
    // treating them as always-changed, so these are safe to emit unconditionally
    // (e.g. when building from a tarball without a .git directory).
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let describe = git_describe().unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=PERCH_GIT_DESCRIBE={describe}");
}

/// Returns `Some("<sha>")` or `Some("<sha>.dirty")` when git is available,
/// `None` when any git invocation fails.
fn git_describe() -> Option<String> {
    let sha = short_sha()?;
    let dirty = is_dirty()?;
    if dirty {
        Some(format!("{sha}.dirty"))
    } else {
        Some(sha)
    }
}

/// Runs `git rev-parse --short=7 HEAD` and returns the trimmed output.
fn short_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    Some(sha.trim().to_owned())
}

/// Runs `git status --porcelain` and returns `true` when the output is
/// non-empty (i.e. there are staged or unstaged changes).
fn is_dirty() -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(!output.stdout.is_empty())
}
