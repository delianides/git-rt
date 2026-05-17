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

/// Branch `b2` forked from `b1` which forked from `main`.
/// resolve_base_branch must return `b1` (the reflog parent), not `main`.
#[test]
fn resolves_reflog_parent_for_stacked_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "base"]);

    // b1 forked from main.
    git(dir, &["checkout", "-b", "b1"]);
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "b1 work"]);

    // b2 forked from b1: use `git branch` so the reflog records "Created from b1",
    // not "Created from HEAD" (which `checkout -b` would write).
    git(dir, &["branch", "b2"]);
    git(dir, &["checkout", "b2"]);

    let repo = GitRepo::new(dir).expect("repo opens");
    assert_eq!(repo.resolve_base_branch(None).as_deref(), Some("b1"));
}

/// Branch `b2` created with implicit `git checkout -b b2` while on `b1`.
/// The branch reflog only says "Created from HEAD"; the parent must be
/// recovered from the HEAD reflog. resolve_base_branch must return `b1`.
#[test]
fn resolves_head_reflog_parent_for_implicit_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "base"]);

    // b1 forked from main.
    git(dir, &["checkout", "-b", "b1"]);
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "b1 work"]);

    // b2 created with implicit `checkout -b` (no start-point) — branch
    // reflog records "Created from HEAD", parent only in the HEAD reflog.
    git(dir, &["checkout", "-b", "b2"]);

    let repo = GitRepo::new(dir).expect("repo opens");
    assert_eq!(repo.resolve_base_branch(None).as_deref(), Some("b1"));
}

/// On a branch with no reflog "Created from" parent (here: `main` itself),
/// resolution falls through to the trunk chain and never picks a sibling.
#[test]
fn trunk_branch_falls_through_to_trunk_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "base"]);

    // A sibling feature branch exists but must never be chosen.
    git(dir, &["branch", "feature-x"]);
    // `git branch` does not switch HEAD, so no `checkout:` entry to
    // `feature-x` is written — the HEAD-reflog tier finds nothing either.

    let repo = GitRepo::new(dir).expect("repo opens");
    // No origin remote, no reflog parent for main -> None (not "feature-x").
    assert_eq!(repo.resolve_base_branch(None), None);
}

/// A re-checkout of a branch must not be mistaken for its creation.
/// `b2` is created off `b1`, then checked out again after visiting `main`;
/// the parent must still resolve to `b1`.
#[test]
fn head_reflog_parent_survives_recheckout() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "base"]);
    git(dir, &["checkout", "-b", "b1"]);
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "b1 work"]);
    git(dir, &["checkout", "-b", "b2"]);
    // Switch away and back — the re-checkout is a later HEAD-reflog entry.
    git(dir, &["checkout", "main"]);
    git(dir, &["checkout", "b2"]);

    let repo = GitRepo::new(dir).expect("repo opens");
    assert_eq!(repo.resolve_base_branch(None).as_deref(), Some("b1"));
}

/// After delete-and-recreate, the stale HEAD-reflog entry from the first
/// `b2` must be ignored: the recreated `b2` was forked from `main`.
#[test]
fn head_reflog_parent_ignores_stale_entry_after_recreate() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "base"]);
    git(dir, &["checkout", "-b", "b1"]);
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "b1 work"]);
    git(dir, &["checkout", "-b", "b2"]); // first b2: off b1
    git(dir, &["checkout", "main"]);
    git(dir, &["branch", "-D", "b2"]);
    git(dir, &["checkout", "-b", "b2"]); // recreated b2: off main

    let repo = GitRepo::new(dir).expect("repo opens");
    assert_eq!(repo.resolve_base_branch(None).as_deref(), Some("main"));
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
