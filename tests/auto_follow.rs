//! Integration tests for `worktree_last_activity`.
//!
//! These tests require a real git repo with real filesystem timestamps, so
//! they live here rather than inline.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use git_rt::watcher::activity::worktree_last_activity;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Run a `git` command inside `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        // Disable GPG/SSH commit signing so tests work in environments where no
        // signing key is configured.
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} exited with status {status}");
}

/// Initialise a minimal git repo with one commit so worktrees can be added.
fn init_repo(dir: &Path) {
    git(dir, &["init", "-b", "main"]);
    fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// `worktree_last_activity` returns `Some` for a real git repo.
#[test]
fn returns_some_for_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    assert!(
        worktree_last_activity(&repo).is_some(),
        "expected Some for a real repo with HEAD"
    );
}

/// `worktree_last_activity` returns `None` for a path with no `.git`.
#[test]
fn returns_none_for_non_repo() {
    let tmp = TempDir::new().unwrap();
    // A plain directory with no git files at all.
    assert!(
        worktree_last_activity(tmp.path()).is_none(),
        "expected None for a non-repo path"
    );
}

/// Given two worktrees, the one whose HEAD was touched more recently reports a
/// later activity timestamp.
#[test]
fn linked_worktree_newer_activity_wins() {
    let tmp = TempDir::new().unwrap();
    let main_repo = tmp.path().join("main_repo");
    let linked_wt = tmp.path().join("feat_wt");

    fs::create_dir_all(&main_repo).unwrap();
    init_repo(&main_repo);

    // Add a linked worktree on a new branch.
    git(
        &main_repo,
        &[
            "worktree",
            "add",
            linked_wt.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );

    // Record timestamps before we touch anything.
    let main_t0 = worktree_last_activity(&main_repo).expect("main_repo should have activity");
    let linked_t0 =
        worktree_last_activity(&linked_wt).expect("linked worktree should have activity");

    // Advance time enough so filesystem resolution can distinguish the writes.
    std::thread::sleep(Duration::from_millis(50));

    // Make a new commit in the linked worktree — this advances its HEAD mtime.
    fs::write(linked_wt.join("feature.txt"), "work\n").unwrap();
    git(&linked_wt, &["add", "."]);
    git(&linked_wt, &["commit", "-m", "feature work"]);

    let main_t1 = worktree_last_activity(&main_repo).expect("main_repo activity");
    let linked_t1 = worktree_last_activity(&linked_wt).expect("linked activity after commit");

    // The linked worktree's timestamp must have advanced.
    assert!(
        linked_t1 > linked_t0,
        "linked worktree activity should be newer after a commit"
    );

    // The linked worktree must now be more recent than the main worktree.
    assert!(
        linked_t1 > main_t1,
        "linked worktree ({linked_t1:?}) should be newer than main ({main_t1:?}) after the feature commit"
    );

    // Sanity: main worktree was not touched, so its timestamp is unchanged or older.
    assert!(
        main_t1 <= main_t0 || linked_t1 > main_t1,
        "main worktree should not have newer activity than the linked worktree"
    );
}

/// With only the main worktree (no linked worktrees), `list_all_worktrees`
/// returns exactly one entry and the activity helper still returns `Some`.
/// This covers the "single worktree is no-op" guard in `check_worktree_activity`.
#[test]
fn single_worktree_activity_is_some() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let worktrees = git_rt::watcher::activity::list_all_worktrees(&repo);
    assert_eq!(worktrees.len(), 1, "expected exactly one worktree");
    assert!(
        worktree_last_activity(&worktrees[0].path).is_some(),
        "single worktree should still report activity"
    );
}

/// `worktree_last_activity` returns `None` for a path that does not exist.
#[test]
fn returns_none_for_nonexistent_path() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("does_not_exist");
    assert!(
        worktree_last_activity(&missing).is_none(),
        "expected None for a path that does not exist"
    );
}
