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
pub fn classify_path(path: &Path) -> PathClass {
    let s = path.to_string_lossy();
    if s.contains("/.git/") {
        if s.ends_with("/.git/HEAD") {
            return PathClass::HeadChange;
        }
        if s.ends_with("/.git/packed-refs") {
            return PathClass::HeadChange;
        }
        if s.contains("/.git/refs/heads/") {
            return PathClass::HeadChange;
        }
        if s.ends_with("/.git/index") {
            return PathClass::FsChange;
        }
        // Remote-side refs: treat as FsChange (refresh status/ahead-behind)
        // but not HeadChange (doesn't move local HEAD).
        if s.contains("/.git/refs/remotes/") {
            return PathClass::FsChange;
        }
        PathClass::Ignored
    } else {
        PathClass::FsChange
    }
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

        // Watch key .git files directly (not all of .git/, which is noisy)
        if let Some(git_dir) = crate::git::resolve_git_dir(repo_path) {
            for candidate in [
                git_dir.join("index"),
                git_dir.join("HEAD"),
                git_dir.join("packed-refs"),
            ] {
                if candidate.exists() {
                    let _ = debouncer.watch(&candidate, RecursiveMode::NonRecursive);
                }
            }
            // Watch refs/heads recursively so both flat and nested branches
            // (e.g., refs/heads/feature/foo) are observed.
            let refs_heads = git_dir.join("refs").join("heads");
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
}
