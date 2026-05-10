//! Tests that confirm git-rt's launch-time behavior matches the
//! pinned-worktree spec.

use std::fs;
use std::path::Path;
use std::process::Command;

use git_rt::git::discover_worktree_root;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} exited with status {status}");
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

#[test]
fn discover_worktree_root_resolves_subdir_to_worktree() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let subdir = repo.join("src").join("nested");
    fs::create_dir_all(&subdir).unwrap();

    let resolved = discover_worktree_root(&subdir).expect("subdir is inside the worktree");
    assert_eq!(
        resolved.canonicalize().unwrap(),
        repo.canonicalize().unwrap(),
        "launching from a subdir should resolve to the worktree root"
    );
}

#[test]
fn discover_worktree_root_resolves_linked_worktree() {
    let tmp = TempDir::new().unwrap();
    let main = tmp.path().join("main");
    let linked = tmp.path().join("linked");
    fs::create_dir_all(&main).unwrap();
    init_repo(&main);
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "feat",
        ],
    );

    let resolved = discover_worktree_root(&linked).expect("linked worktree path is valid");
    assert_eq!(
        resolved.canonicalize().unwrap(),
        linked.canonicalize().unwrap(),
        "launching in a linked worktree should resolve to that worktree, not the main one"
    );
}
