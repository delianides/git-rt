pub mod activity;
pub mod worktree;

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
        let mut debouncer = new_debouncer(
            debounce,
            None,
            move |result: Result<Vec<DebouncedEvent>, Vec<notify::Error>>| {
                match result {
                    Ok(events) => {
                        // Filter out .git directory changes (except index)
                        let relevant = events
                            .iter()
                            .any(|e| e.event.paths.iter().any(|p| is_relevant_path(p)));

                        tracing::debug!(
                            event_count = events.len(),
                            relevant,
                            "Debouncer callback fired"
                        );

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
            },
        )
        .context("Failed to create filesystem debouncer")?;

        debouncer
            .watch(repo_path, RecursiveMode::Recursive)
            .context("Failed to watch repository path")?;

        // Also watch the git index specifically for staging changes.
        // In a linked worktree, .git is a file pointing to the real gitdir,
        // so we resolve it first.
        if let Some(git_dir) = crate::git::resolve_git_dir(repo_path) {
            let git_index = git_dir.join("index");
            if git_index.exists() {
                let _ = debouncer.watch(&git_index, RecursiveMode::NonRecursive);
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

/// Returns true if the given path represents a relevant change
/// (i.e., not inside .git/ except for paths that indicate
/// index changes, commits, or branch switches)
fn is_relevant_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    if path_str.contains("/.git/") {
        // .git/index — staging changes
        // .git/refs/ — commits (refs/heads), tags, remote updates
        // .git/HEAD — branch switches, detached HEAD changes
        path_str.ends_with("/.git/index")
            || path_str.contains("/.git/refs/")
            || path_str.ends_with("/.git/HEAD")
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regular_file_is_relevant() {
        assert!(is_relevant_path(Path::new("/repo/src/main.rs")));
        assert!(is_relevant_path(Path::new("/repo/Cargo.toml")));
        assert!(is_relevant_path(Path::new("/repo/README.md")));
    }

    #[test]
    fn test_git_internal_files_not_relevant() {
        assert!(!is_relevant_path(Path::new("/repo/.git/objects/ab/cd1234")));
        assert!(!is_relevant_path(Path::new("/repo/.git/COMMIT_EDITMSG")));
        assert!(!is_relevant_path(Path::new("/repo/.git/logs/HEAD")));
        assert!(!is_relevant_path(Path::new("/repo/.git/config")));
        assert!(!is_relevant_path(Path::new("/repo/.git/MERGE_HEAD")));
    }

    #[test]
    fn test_git_index_is_relevant() {
        assert!(is_relevant_path(Path::new("/repo/.git/index")));
    }

    #[test]
    fn test_git_index_lock_not_relevant() {
        assert!(!is_relevant_path(Path::new("/repo/.git/index.lock")));
    }

    #[test]
    fn test_git_refs_are_relevant() {
        assert!(is_relevant_path(Path::new("/repo/.git/refs/heads/main")));
        assert!(is_relevant_path(Path::new(
            "/repo/.git/refs/heads/feature/foo"
        )));
        assert!(is_relevant_path(Path::new("/repo/.git/refs/tags/v1.0")));
        assert!(is_relevant_path(Path::new(
            "/repo/.git/refs/remotes/origin/main"
        )));
    }

    #[test]
    fn test_git_head_is_relevant() {
        assert!(is_relevant_path(Path::new("/repo/.git/HEAD")));
    }

    #[test]
    fn test_file_named_git_outside_dotgit_is_relevant() {
        // A file that happens to have "git" in the name but isn't in .git/
        assert!(is_relevant_path(Path::new("/repo/src/git/mod.rs")));
        assert!(is_relevant_path(Path::new("/repo/.gitignore")));
    }
}
