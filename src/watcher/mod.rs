pub mod activity;
pub mod worktree;

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebouncedEvent, Debouncer, RecommendedCache};

/// Classification of a filesystem path that fired a debounced event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    /// Working-tree file or `.git/index` — recompute git status.
    FsChange,
    /// `.git/HEAD`, current branch ref, or `packed-refs` — recompute commits.
    HeadChange,
    /// Noise (e.g., `.git/config`, `.git/objects/*`, `index.lock`).
    Ignored,
}

/// High-level event type emitted to the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsWatcherEvent {
    FsChange,
    HeadChange,
}

/// Classify a single path into one of the three categories.
///
/// Recognizes both main-gitdir paths (`.git/HEAD`, `.git/index`, `.git/refs/heads/…`)
/// and linked-worktree gitdir paths (`.git/worktrees/<name>/HEAD`,
/// `.git/worktrees/<name>/index`). Linked worktrees store their own `HEAD`
/// and `index` under `.git/worktrees/<name>/` while sharing `refs/heads/`
/// and `packed-refs` with the common (main) gitdir.
pub fn classify_path(path: &Path) -> PathClass {
    let s = path.to_string_lossy();
    if !s.contains("/.git/") {
        return PathClass::FsChange;
    }

    // Reflog files are named like real refs (e.g. `.git/logs/HEAD`) but are
    // write-only history, not live state — always ignore them.
    if s.contains("/.git/logs/") {
        return PathClass::Ignored;
    }

    // HEAD files: main gitdir OR linked worktree gitdir.
    //   .git/HEAD                        → main worktree's HEAD
    //   .git/worktrees/<name>/HEAD       → linked worktree's HEAD
    if s.ends_with("/.git/HEAD") || (s.contains("/.git/worktrees/") && s.ends_with("/HEAD")) {
        return PathClass::HeadChange;
    }

    // packed-refs lives in the common gitdir and is shared across worktrees.
    if s.ends_with("/.git/packed-refs") {
        return PathClass::HeadChange;
    }

    // Branch refs live in the common gitdir; shared by all worktrees.
    if s.contains("/.git/refs/heads/") {
        return PathClass::HeadChange;
    }

    // index files: main gitdir OR linked worktree gitdir.
    //   .git/index                       → main worktree's staging index
    //   .git/worktrees/<name>/index      → linked worktree's staging index
    // Either firing should trigger a `git.status()` recompute so that
    // freshly-committed files disappear from the Changes tab.
    if s.ends_with("/.git/index") || (s.contains("/.git/worktrees/") && s.ends_with("/index")) {
        return PathClass::FsChange;
    }

    // Remote-side refs: treat as FsChange (refresh status/ahead-behind)
    // but not HeadChange (doesn't move local HEAD).
    if s.contains("/.git/refs/remotes/") {
        return PathClass::FsChange;
    }

    PathClass::Ignored
}

/// Filesystem watcher that sends debounced change notifications
pub struct FsWatcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl FsWatcher {
    /// Create a new filesystem watcher for the given repo path.
    ///
    /// Returns a receiver that yields `FsWatcherEvent` for each debounced
    /// change event, and the watcher handle (which must be kept alive).
    ///
    /// A single debounced batch can produce up to two events — a `HeadChange`
    /// **and** an `FsChange` — if the batch contains both kinds of paths.
    /// `HeadChange` is sent first so the app can recompute the commits list
    /// before rescanning the working tree.
    pub fn new(repo_path: &Path, debounce: Duration) -> Result<(Receiver<FsWatcherEvent>, Self)> {
        let (tx, rx) = bounded::<FsWatcherEvent>(16);

        let sender = tx.clone();
        let mut debouncer = new_debouncer(
            debounce,
            None,
            move |result: Result<Vec<DebouncedEvent>, Vec<notify::Error>>| match result {
                Ok(events) => {
                    let mut has_fs = false;
                    let mut has_head = false;
                    for e in &events {
                        for p in &e.event.paths {
                            match classify_path(p) {
                                PathClass::FsChange => has_fs = true,
                                PathClass::HeadChange => has_head = true,
                                PathClass::Ignored => {}
                            }
                        }
                    }
                    tracing::debug!(
                        event_count = events.len(),
                        has_fs,
                        has_head,
                        "Debouncer callback fired"
                    );
                    if has_head {
                        let _ = sender.try_send(FsWatcherEvent::HeadChange);
                    }
                    if has_fs {
                        let _ = sender.try_send(FsWatcherEvent::FsChange);
                    }
                }
                Err(errors) => {
                    for e in errors {
                        tracing::warn!("Filesystem watch error: {e}");
                    }
                }
            },
        )
        .context("Failed to create filesystem debouncer")?;

        debouncer
            .watch(repo_path, RecursiveMode::Recursive)
            .context("Failed to watch repository path")?;

        // Watch key .git files directly (not all of .git/, which is noisy).
        //
        // Notes on linked worktrees:
        //   - `resolve_git_dir` returns the **worktree-specific** gitdir,
        //     e.g. `<main>/.git/worktrees/<name>/`. The worktree-specific
        //     `HEAD` and `index` live there and must be watched directly.
        //   - `resolve_common_git_dir` returns the **common** gitdir
        //     (`<main>/.git/`). `refs/heads/<branch>` and `packed-refs`
        //     live there and are shared across all worktrees. They must
        //     also be watched so branch-ref advances in a linked worktree
        //     are observed.
        //   - For a main worktree the two resolvers return the same path;
        //     the `!=` guard avoids redundant watches without duplicating
        //     setup branches.
        if let Some(git_dir) = crate::git::resolve_git_dir(repo_path) {
            // Worktree-specific files: HEAD + index. packed-refs is NOT in
            // the worktree-specific dir (only in common) — skip it here.
            for candidate in [git_dir.join("index"), git_dir.join("HEAD")] {
                if candidate.exists() {
                    let _ = debouncer.watch(&candidate, RecursiveMode::NonRecursive);
                }
            }
        }
        if let Some(common_dir) = crate::git::resolve_common_git_dir(repo_path) {
            // packed-refs: may or may not exist depending on `git gc` state.
            let packed_refs = common_dir.join("packed-refs");
            if packed_refs.exists() {
                let _ = debouncer.watch(&packed_refs, RecursiveMode::NonRecursive);
            }
            // refs/heads: watched recursively so nested branches
            // (e.g., refs/heads/feature/foo) are observed.
            let refs_heads = common_dir.join("refs").join("heads");
            if refs_heads.exists() {
                let _ = debouncer.watch(&refs_heads, RecursiveMode::Recursive);
            }
        }

        tracing::info!(?repo_path, "Filesystem watcher started");

        Ok((
            rx,
            Self {
                _debouncer: debouncer,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_head_path() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/HEAD")),
            PathClass::HeadChange
        );
    }

    #[test]
    fn test_classify_branch_ref_path() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/refs/heads/main")),
            PathClass::HeadChange
        );
        assert_eq!(
            classify_path(Path::new("/repo/.git/refs/heads/feature/foo")),
            PathClass::HeadChange
        );
    }

    #[test]
    fn test_classify_packed_refs_path() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/packed-refs")),
            PathClass::HeadChange
        );
    }

    #[test]
    fn test_classify_index_is_fs_change() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/index")),
            PathClass::FsChange
        );
    }

    #[test]
    fn test_classify_working_tree_file_is_fs_change() {
        assert_eq!(
            classify_path(Path::new("/repo/src/main.rs")),
            PathClass::FsChange
        );
    }

    #[test]
    fn test_classify_remote_ref_is_fs_change() {
        // refs/remotes/* changes from fetch — treat as fs change, not head change
        assert_eq!(
            classify_path(Path::new("/repo/.git/refs/remotes/origin/main")),
            PathClass::FsChange
        );
    }

    #[test]
    fn test_classify_git_config_is_ignored() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/config")),
            PathClass::Ignored
        );
    }

    // --- Linked worktree gitdir paths --------------------------------------
    //
    // For a linked worktree, `HEAD` and `index` live in a per-worktree
    // subdirectory under `.git/worktrees/<name>/`, not directly under `.git/`.
    // These cases exist for the previous classifier regression where a
    // commit inside a linked worktree failed to refresh the Changes tab
    // because `.git/worktrees/<name>/index` was classified as Ignored.

    #[test]
    fn test_classify_linked_worktree_index_is_fs_change() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/worktrees/feat/index")),
            PathClass::FsChange
        );
        // Nested worktree name (git allows `git worktree add` with nested
        // names via the internal `-` separator; the on-disk path can still
        // use slashes through the `.git/worktrees/<slug>/` dir).
        assert_eq!(
            classify_path(Path::new("/repo/.git/worktrees/drew-tabbed-view/index")),
            PathClass::FsChange
        );
    }

    #[test]
    fn test_classify_linked_worktree_head_is_head_change() {
        assert_eq!(
            classify_path(Path::new("/repo/.git/worktrees/feat/HEAD")),
            PathClass::HeadChange
        );
    }

    #[test]
    fn test_classify_reflog_is_ignored() {
        // `.git/logs/HEAD` is a reflog, not a real HEAD update — should be
        // Ignored even though the filename is `HEAD`.
        assert_eq!(
            classify_path(Path::new("/repo/.git/logs/HEAD")),
            PathClass::Ignored
        );
        assert_eq!(
            classify_path(Path::new("/repo/.git/logs/refs/heads/main")),
            PathClass::Ignored
        );
    }

    #[test]
    fn test_classify_linked_worktree_commondir_file_is_ignored() {
        // .git/worktrees/<name>/commondir — internal metadata, not a trigger.
        assert_eq!(
            classify_path(Path::new("/repo/.git/worktrees/feat/commondir")),
            PathClass::Ignored
        );
        assert_eq!(
            classify_path(Path::new("/repo/.git/worktrees/feat/gitdir")),
            PathClass::Ignored
        );
    }
}
