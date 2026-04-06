use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebouncedEvent, Debouncer, RecommendedCache};

/// Filesystem watcher that sends debounced change notifications
pub struct FsWatcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl FsWatcher {
    /// Create a new filesystem watcher for the given repo path.
    /// Returns a receiver that gets `()` on each debounced change event,
    /// and the watcher handle (must be kept alive).
    pub fn new(repo_path: &Path, debounce: Duration) -> Result<(Receiver<()>, Self)> {
        let (tx, rx) = bounded::<()>(16);

        let sender = tx.clone();
        let mut debouncer = new_debouncer(debounce, None, move |result: Result<Vec<DebouncedEvent>, Vec<notify::Error>>| {
            match result {
                Ok(events) => {
                    // Filter out .git directory changes (except index)
                    let relevant = events.iter().any(|e| {
                        e.event.paths.iter().any(|p| {
                            let path_str = p.to_string_lossy();
                            if path_str.contains("/.git/") {
                                // Only care about index changes (staging)
                                path_str.ends_with("/.git/index")
                            } else {
                                true
                            }
                        })
                    });

                    if relevant {
                        // Non-blocking send — drop if channel is full
                        let _ = sender.try_send(());
                    }
                }
                Err(errors) => {
                    for e in errors {
                        tracing::warn!("Filesystem watch error: {e}");
                    }
                }
            }
        })
        .context("Failed to create filesystem debouncer")?;

        debouncer
            .watch(repo_path, RecursiveMode::Recursive)
            .context("Failed to watch repository path")?;

        // Also watch .git/index specifically for staging changes
        let git_index = repo_path.join(".git/index");
        if git_index.exists() {
            let _ = debouncer
                .watch(&git_index, RecursiveMode::NonRecursive);
        }

        tracing::info!(?repo_path, "Filesystem watcher started");

        Ok((rx, Self {
            _debouncer: debouncer,
        }))
    }
}
