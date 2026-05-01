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

fn init_repo(tmp: &std::path::Path) -> std::path::PathBuf {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "test"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-q", "-m", "init"]);
    repo
}

#[test]
fn list_returns_main_and_added_worktrees() {
    let tmp = tempdir().unwrap();
    let repo = init_repo(tmp.path());

    // Add a second worktree on a new branch.
    let wt2 = repo.join(".worktrees").join("feat");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat-x",
            wt2.to_str().unwrap(),
        ],
    );

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

#[test]
fn list_reports_locked_worktree() {
    let tmp = tempdir().unwrap();
    let repo = init_repo(tmp.path());

    // Add two extra worktrees on feat-x and feat-y.
    let wt_feat_x = repo.join(".worktrees").join("feat-x");
    let wt_feat_y = repo.join(".worktrees").join("feat-y");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat-x",
            wt_feat_x.to_str().unwrap(),
        ],
    );
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat-y",
            wt_feat_y.to_str().unwrap(),
        ],
    );

    // Lock the feat-y worktree with a reason.
    run_git(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "test lock",
            wt_feat_y.to_str().unwrap(),
        ],
    );

    let entries = list(&repo).expect("list() failed");
    assert_eq!(
        entries.len(),
        3,
        "expected 3 entries (main + feat-x + feat-y)"
    );

    // Find each entry by branch.
    let find = |branch: &str| {
        entries
            .iter()
            .find(|e| e.branch.as_deref() == Some(branch))
            .unwrap_or_else(|| panic!("no entry for branch {branch}"))
    };

    let entry_feat_y = find("feat-y");
    assert_eq!(
        entry_feat_y.locked.as_deref(),
        Some("test lock"),
        "feat-y should be locked with reason 'test lock', got {:?}",
        entry_feat_y.locked
    );

    let entry_feat_x = find("feat-x");
    assert!(
        entry_feat_x.locked.is_none(),
        "feat-x should not be locked, got {:?}",
        entry_feat_x.locked
    );

    let entry_main = find("main");
    assert!(
        entry_main.locked.is_none(),
        "main should not be locked, got {:?}",
        entry_main.locked
    );
}
