//! End-to-end test for `git::worktree::list` against a real git repo.

use std::process::Command;

use tempfile::tempdir;

use git_rt::git::worktree::list;

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git command failed to start");
    assert!(status.success(), "git {args:?} failed in {cwd:?}");
}

#[test]
fn list_returns_main_and_added_worktrees() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    // git init + initial commit on `main`.
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "test"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-q", "-m", "init"]);

    // Add a second worktree on a new branch.
    let wt2 = repo.join(".worktrees").join("feat");
    run_git(&repo, &["worktree", "add", "-q", "-b", "feat-x", wt2.to_str().unwrap()]);

    let entries = list(&repo).expect("list() failed");
    assert_eq!(entries.len(), 2);

    let branches: Vec<_> = entries.iter().filter_map(|e| e.branch.as_deref()).collect();
    assert!(branches.contains(&"main"), "got {branches:?}");
    assert!(branches.contains(&"feat-x"), "got {branches:?}");

    // Every entry should have a 40-hex SHA.
    for e in &entries {
        assert_eq!(e.head.len(), 40, "head sha should be 40 hex chars: {e:?}");
        assert!(!e.bare);
        assert!(!e.detached);
        assert!(e.prunable.is_none());
    }
}
