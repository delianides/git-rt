//! End-to-end integration tests for trunk-priority base resolution.
//!
//! Builds real git repos with `tempfile` + shell-out to `git`, then calls
//! `GitRepo::resolve_base_branch` to verify the priority chain.

use std::path::Path;
use std::process::Command;

use perch::git::GitRepo;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .status()
        .expect("git command must run");
    assert!(status.success(), "git {:?} failed", args);
}

/// Repo with origin/HEAD → main; resolve returns "main".
#[test]
fn resolves_origin_head() {
    let tmp = tempfile::tempdir().unwrap();
    let upstream = tmp.path().join("upstream.git");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    Command::new("git")
        .args(["init", "-q", "--bare", upstream.to_str().unwrap()])
        .status()
        .unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a"), "x").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "m1"]);
    git(
        &work,
        &["remote", "add", "origin", upstream.to_str().unwrap()],
    );
    git(&work, &["push", "-q", "-u", "origin", "main"]);
    git(&work, &["remote", "set-head", "origin", "main"]);

    let repo = GitRepo::new(&work).unwrap();
    assert_eq!(repo.resolve_base_branch(None).as_deref(), Some("main"));
}

/// Sibling branches are NOT chosen even when their merge-base is closer.
#[test]
fn ignores_sibling_branches() {
    let tmp = tempfile::tempdir().unwrap();
    let upstream = tmp.path().join("upstream.git");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    Command::new("git")
        .args(["init", "-q", "--bare", upstream.to_str().unwrap()])
        .status()
        .unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a"), "x").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "m1"]);
    git(
        &work,
        &["remote", "add", "origin", upstream.to_str().unwrap()],
    );
    git(&work, &["push", "-q", "-u", "origin", "main"]);
    git(&work, &["remote", "set-head", "origin", "main"]);
    // Two sibling branches both rooted at the latest main commit
    git(&work, &["branch", "drew/sibling-a"]);
    git(&work, &["checkout", "-q", "-b", "drew/sibling-b"]);
    std::fs::write(work.join("b"), "y").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "b1"]);

    let repo = GitRepo::new(&work).unwrap();
    let base = repo.resolve_base_branch(None);
    assert_eq!(
        base.as_deref(),
        Some("main"),
        "expected trunk, got sibling-aware: {:?}",
        base
    );
}

/// Explicit override wins over auto-detection.
#[test]
fn explicit_override_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a"), "x").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "m1"]);

    let repo = GitRepo::new(&work).unwrap();
    assert_eq!(
        repo.resolve_base_branch(Some("custom")).as_deref(),
        Some("custom")
    );
}

/// Repo with no remote falls through to None.
#[test]
fn no_remote_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a"), "x").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "m1"]);

    let repo = GitRepo::new(&work).unwrap();
    assert_eq!(repo.resolve_base_branch(None), None);
}
