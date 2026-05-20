//! End-to-end integration tests for strict default-branch base resolution.
//!
//! Builds real git repos with `tempfile` + shell-out to `git`, then calls
//! `GitRepo::resolve_base_branch` to verify the strict priority order.

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

/// Linked worktrees store remote refs in the common git dir; strict resolution
/// must still find origin/HEAD there.
#[test]
fn resolves_origin_head_from_linked_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let upstream = tmp.path().join("upstream.git");
    let work = tmp.path().join("work");
    let linked = tmp.path().join("linked");
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
    git(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
            "main",
        ],
    );

    let repo = GitRepo::new(&linked).unwrap();
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
        "expected default branch, got sibling-aware: {:?}",
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
/// Without origin/HEAD, strict resolution must not infer `b1` from reflog data.
#[test]
fn ignores_branch_reflog_parent_for_stacked_branch_without_origin_head() {
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

    git(dir, &["branch", "b2"]);
    git(dir, &["checkout", "b2"]);

    let repo = GitRepo::new(dir).expect("repo opens");
    assert_eq!(repo.resolve_base_branch(None), None);
}

/// Branch `b2` created with implicit `git checkout -b b2` while on `b1`.
/// Strict resolution must not recover `b1` from the HEAD reflog.
#[test]
fn ignores_head_reflog_parent_for_implicit_checkout_without_origin_head() {
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

    let repo = GitRepo::new(dir).expect("repo opens");
    assert_eq!(repo.resolve_base_branch(None), None);
}

/// Even when local or remote main/master-looking refs exist, strict resolution
/// only accepts origin/HEAD as the automatic repository default.
#[test]
fn ignores_main_and_master_refs_without_origin_head() {
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
    git(&work, &["branch", "master"]);
    git(&work, &["push", "-q", "origin", "master"]);

    let origin_head = work.join(".git/refs/remotes/origin/HEAD");
    let _ = std::fs::remove_file(origin_head);

    let repo = GitRepo::new(&work).unwrap();
    assert_eq!(repo.resolve_base_branch(None), None);
}

/// On a branch without origin/HEAD, strict resolution never picks a sibling.
#[test]
fn branch_without_origin_head_ignores_sibling() {
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
    // No origin/HEAD -> None (not "feature-x").
    assert_eq!(repo.resolve_base_branch(None), None);
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
