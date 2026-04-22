//! End-to-end integration tests for per-branch base detection.
//!
//! Builds real git repos with `tempfile` + shell-out to `git`, then calls
//! `GitRepo::detect_base_branch` directly to verify the full tier-1 + tier-2
//! flow produces the expected result.

use std::path::Path;
use std::process::Command;

use git_rt::git::GitRepo;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .expect("git command must run");
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn stacked_branches_detect_nearest_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    // main ← feature-a ← feature-b
    git(p, &["init", "-q", "-b", "main"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ],
    );
    git(p, &["checkout", "-q", "-b", "feature-a"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "a1",
        ],
    );
    git(p, &["checkout", "-q", "-b", "feature-b"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "b1",
        ],
    );

    let repo = GitRepo::new(p).unwrap();
    assert_eq!(
        repo.detect_base_branch("feature-b"),
        Some("feature-a".to_string())
    );
}

#[test]
fn branch_off_remote_tracking_returns_short_name() {
    // Create a "remote" bare repo with develop, then clone it, then branch
    // off origin/develop without checking it out locally.
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let clone = tmp.path().join("clone");

    let status = Command::new("git")
        .args(["init", "-q", "--bare", "-b", "main"])
        .arg(&remote)
        .status()
        .expect("git init bare");
    assert!(status.success(), "bare init failed");

    // Seed the bare repo via a scratch clone.
    let scratch = tmp.path().join("scratch");
    let status = Command::new("git")
        .args(["clone", "-q"])
        .arg(&remote)
        .arg(&scratch)
        .status()
        .expect("git clone scratch");
    assert!(status.success(), "scratch clone failed");

    git(
        &scratch,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ],
    );
    git(&scratch, &["checkout", "-q", "-b", "develop"]);
    git(
        &scratch,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "d1",
        ],
    );
    git(&scratch, &["push", "-q", "origin", "main", "develop"]);

    // Now clone fresh and branch off origin/develop without checking it out.
    let status = Command::new("git")
        .args(["clone", "-q"])
        .arg(&remote)
        .arg(&clone)
        .status()
        .expect("git clone");
    assert!(status.success(), "clone failed");

    git(
        &clone,
        &["checkout", "-q", "-b", "feature", "origin/develop"],
    );
    git(
        &clone,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "f1",
        ],
    );

    let repo = GitRepo::new(&clone).unwrap();
    // Tier 1 should pick "develop" (short name) from reflog "Created from origin/develop".
    assert_eq!(
        repo.detect_base_branch("feature"),
        Some("develop".to_string())
    );
}

#[test]
fn explicit_base_override_short_circuits_detection() {
    // Verify at the compute_status / worker integration level: explicit
    // override wins even when detection would pick something else.
    use crossbeam_channel::bounded;
    use git_rt::git::worker::{Request, Response, Worker};
    use std::time::Duration;

    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ],
    );
    git(p, &["checkout", "-q", "-b", "feature-a"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "a1",
        ],
    );
    git(p, &["checkout", "-q", "-b", "feature-b", "feature-a"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "b1",
        ],
    );

    let (req_tx, req_rx) = bounded::<Request>(8);
    let (resp_tx, resp_rx) = bounded::<Response>(8);
    let handle = Worker::spawn(
        p.to_path_buf(),
        Some("main".to_string()), // explicit --base main
        None,
        req_rx,
        resp_tx,
    );

    req_tx.send(Request::Recompute).unwrap();
    let resp = resp_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    match resp {
        Response::Status(b) => assert_eq!(b.base_branch, "main"),
        other => panic!("expected Status, got {:?}", other),
    }

    req_tx.send(Request::Shutdown).unwrap();
    handle.join().unwrap();
}

#[test]
fn rebased_branch_detects_new_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    // main ← feature-a ← feature-b (initially)
    git(p, &["init", "-q", "-b", "main"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "m1",
        ],
    );
    git(p, &["checkout", "-q", "-b", "feature-a"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "a1",
        ],
    );
    git(p, &["checkout", "-q", "-b", "feature-b"]);
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "b1",
        ],
    );

    // Delete the reflog so tier-2 (merge-base) drives the result.
    std::fs::remove_file(p.join(".git/logs/refs/heads/feature-b")).ok();

    // Rebase feature-b directly onto main (dropping feature-a's commit).
    git(
        p,
        &[
            "-c",
            "commit.gpgsign=false",
            "rebase",
            "-q",
            "--onto",
            "main",
            "feature-a",
            "feature-b",
        ],
    );

    let repo = GitRepo::new(p).unwrap();
    assert_eq!(
        repo.detect_base_branch("feature-b"),
        Some("main".to_string())
    );
}
