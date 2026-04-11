//! Commit range walking for the Commits tab. Types here are referenced by
//! `CommitsTabState` in `src/state/mod.rs`. The walker functions are
//! added in a later task.

/// A single commit entry for the Commits tab list.
#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub sha_full: String,
    pub sha_short: String,
    pub title: String,
}
