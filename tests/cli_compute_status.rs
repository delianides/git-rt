use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

use git_rt::git::cli::compute_status_files;
use git_rt::git::FileStatus;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed");
    assert!(
        out.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@test.com"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

#[test]
fn compute_status_files_no_base_returns_uncommitted_and_untracked() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    init_repo(repo);
    std::fs::write(repo.join("a.txt"), "v1\n").unwrap();
    git(repo, &["add", "a.txt"]);
    git(repo, &["commit", "-q", "-m", "initial"]);

    // Modify a tracked file (unstaged), and add an untracked file.
    std::fs::write(repo.join("a.txt"), "v1\nedit\n").unwrap();
    std::fs::write(repo.join("new.txt"), "hello\n").unwrap();

    let entries = compute_status_files(repo, None).unwrap();
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(paths.contains(&"new.txt"));

    let a = entries.iter().find(|e| e.path == "a.txt").unwrap();
    assert!(matches!(a.status, FileStatus::Modified));
    assert!(a.insertions >= 1);

    let n = entries.iter().find(|e| e.path == "new.txt").unwrap();
    assert!(matches!(n.status, FileStatus::Untracked));
    assert_eq!(n.insertions, 1);
}

#[test]
fn compute_status_files_with_base_includes_committed_changes() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    init_repo(repo);
    std::fs::write(repo.join("base.txt"), "base\n").unwrap();
    git(repo, &["add", "base.txt"]);
    git(repo, &["commit", "-q", "-m", "initial"]);

    git(repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("committed.txt"), "from-feature\n").unwrap();
    git(repo, &["add", "committed.txt"]);
    git(repo, &["commit", "-q", "-m", "add committed"]);

    // Resolve merge base to the initial commit on main.
    let mb_output = Command::new("git")
        .args(["merge-base", "main", "HEAD"])
        .current_dir(repo)
        .output()
        .unwrap();
    let mb_hex = String::from_utf8(mb_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    let mb_oid = gix::ObjectId::from_hex(mb_hex.as_bytes()).unwrap();

    // Add an uncommitted edit and an untracked file too.
    std::fs::write(repo.join("base.txt"), "base\nedit\n").unwrap();
    std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();

    let entries = compute_status_files(repo, Some(&mb_oid)).unwrap();
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();

    assert!(
        paths.contains(&"committed.txt"),
        "committed-on-branch must appear"
    );
    assert!(paths.contains(&"base.txt"), "uncommitted edit must appear");
    assert!(paths.contains(&"untracked.txt"), "untracked must appear");
}
