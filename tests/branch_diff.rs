use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// Helper to run git commands in a directory
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {:?} failed (exit {}): {}", args, output.status, stderr);
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn git_init(dir: &Path) {
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "test@test.com"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

#[test]
fn test_branch_diff_shows_committed_files() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();

    git_init(repo_path);
    std::fs::write(repo_path.join("README.md"), "# Hello\n").unwrap();
    git(repo_path, &["add", "README.md"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);
    std::fs::write(repo_path.join("new_file.rs"), "fn main() {}\n").unwrap();
    git(repo_path, &["add", "new_file.rs"]);
    git(repo_path, &["commit", "-m", "add new file"]);

    std::fs::write(repo_path.join("README.md"), "# Hello\n\nUpdated.\n").unwrap();
    git(repo_path, &["add", "README.md"]);
    git(repo_path, &["commit", "-m", "update readme"]);

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();

    let mb = git_repo
        .merge_base("main")
        .unwrap()
        .expect("should find merge base");

    let entries = git_repo.branch_status(mb).unwrap();
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();

    assert!(
        paths.contains(&"new_file.rs"),
        "new_file.rs should be in branch diff"
    );
    assert!(
        paths.contains(&"README.md"),
        "README.md should be in branch diff"
    );

    let new_file = entries.iter().find(|e| e.path == "new_file.rs").unwrap();
    assert!(matches!(new_file.status, perch::git::FileStatus::Added));
    assert!(new_file.insertions > 0);

    let readme = entries.iter().find(|e| e.path == "README.md").unwrap();
    assert!(matches!(readme.status, perch::git::FileStatus::Modified));
}

#[test]
fn test_branch_diff_includes_uncommitted_changes() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();

    git_init(repo_path);
    std::fs::write(repo_path.join("base.txt"), "base\n").unwrap();
    git(repo_path, &["add", "base.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);

    std::fs::write(repo_path.join("committed.txt"), "committed\n").unwrap();
    git(repo_path, &["add", "committed.txt"]);
    git(repo_path, &["commit", "-m", "add committed"]);

    // Uncommitted change
    std::fs::write(repo_path.join("uncommitted.txt"), "uncommitted\n").unwrap();

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let mb = git_repo.merge_base("main").unwrap().expect("merge base");
    let entries = git_repo.branch_status(mb).unwrap();
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();

    assert!(paths.contains(&"committed.txt"));
    assert!(paths.contains(&"uncommitted.txt"));
}

#[test]
fn test_merge_base_none_on_default_branch() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();

    git_init(repo_path);
    std::fs::write(repo_path.join("file.txt"), "content\n").unwrap();
    git(repo_path, &["add", "file.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let result = git_repo.merge_base("main").unwrap();
    assert!(
        result.is_none(),
        "merge_base should be None on the base branch itself"
    );
}

#[test]
fn test_branch_diff_file_shows_full_diff() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();

    git_init(repo_path);
    std::fs::write(repo_path.join("file.txt"), "line1\nline2\nline3\n").unwrap();
    git(repo_path, &["add", "file.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);
    std::fs::write(
        repo_path.join("file.txt"),
        "line1\nmodified\nline3\nnew_line\n",
    )
    .unwrap();
    git(repo_path, &["add", "file.txt"]);
    git(repo_path, &["commit", "-m", "modify file"]);

    std::fs::write(
        repo_path.join("file.txt"),
        "line1\nmodified\nline3\nnew_line\nextra\n",
    )
    .unwrap();

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let mb = git_repo.merge_base("main").unwrap().expect("merge base");
    let diff = git_repo.branch_diff_file("file.txt", mb).unwrap();

    assert!(!diff.hunks.is_empty(), "diff should have hunks");

    let additions: usize = diff
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l.kind, perch::git::DiffLineKind::Addition))
        .count();
    let deletions: usize = diff
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l.kind, perch::git::DiffLineKind::Deletion))
        .count();

    assert!(additions >= 2, "should have additions");
    assert!(deletions >= 1, "should have deletions");
}

#[test]
fn test_branch_status_only_committed_changes() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();
    git_init(repo_path);
    std::fs::write(repo_path.join("a.txt"), "a\n").unwrap();
    git(repo_path, &["add", "a.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);
    std::fs::write(repo_path.join("a.txt"), "a\nmodified\n").unwrap();
    std::fs::write(repo_path.join("b.txt"), "b\n").unwrap();
    git(repo_path, &["add", "."]);
    git(repo_path, &["commit", "-m", "feature commit"]);

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let mb = git_repo.merge_base("main").unwrap().expect("merge base");
    let entries = git_repo.branch_status(mb).unwrap();

    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"), "a.txt (modified) must appear");
    assert!(paths.contains(&"b.txt"), "b.txt (added) must appear");

    let a = entries.iter().find(|e| e.path == "a.txt").unwrap();
    assert!(matches!(a.status, perch::git::FileStatus::Modified));
    assert!(a.insertions > 0, "modified file should have insertions");

    let b = entries.iter().find(|e| e.path == "b.txt").unwrap();
    assert!(matches!(b.status, perch::git::FileStatus::Added));
    assert!(b.insertions > 0, "added file should have insertions");
}

#[test]
fn test_branch_status_only_uncommitted_changes() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();
    git_init(repo_path);
    std::fs::write(repo_path.join("a.txt"), "a\n").unwrap();
    git(repo_path, &["add", "a.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);
    std::fs::write(repo_path.join("b.txt"), "b\n").unwrap();
    git(repo_path, &["add", "b.txt"]);
    git(repo_path, &["commit", "-m", "add b"]);

    std::fs::write(repo_path.join("a.txt"), "a\nedited\n").unwrap();
    std::fs::write(repo_path.join("untracked.txt"), "u\n").unwrap();

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let mb = git_repo.merge_base("main").unwrap().expect("merge base");
    let entries = git_repo.branch_status(mb).unwrap();
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();

    assert!(paths.contains(&"a.txt"), "edited tracked file must appear");
    assert!(
        paths.contains(&"untracked.txt"),
        "untracked file must appear"
    );

    let untracked = entries.iter().find(|e| e.path == "untracked.txt").unwrap();
    assert!(
        matches!(untracked.status, perch::git::FileStatus::Untracked),
        "untracked file must keep Untracked status, got {:?}",
        untracked.status
    );
}

#[test]
fn test_branch_status_committed_then_further_edited() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();
    git_init(repo_path);
    std::fs::write(repo_path.join("a.txt"), "v0\n").unwrap();
    git(repo_path, &["add", "a.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);
    std::fs::write(repo_path.join("a.txt"), "v1\n").unwrap();
    git(repo_path, &["add", "a.txt"]);
    git(repo_path, &["commit", "-m", "edit a"]);

    std::fs::write(repo_path.join("a.txt"), "v2\nextra\n").unwrap();

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let mb = git_repo.merge_base("main").unwrap().expect("merge base");
    let entries = git_repo.branch_status(mb).unwrap();

    let a = entries
        .iter()
        .find(|e| e.path == "a.txt")
        .expect("a.txt must appear");
    assert!(
        a.insertions >= 2,
        "expected ≥2 insertions vs merge base; got {}",
        a.insertions
    );
}

#[test]
fn test_branch_status_deleted_on_branch() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path();
    git_init(repo_path);
    std::fs::write(repo_path.join("a.txt"), "line1\nline2\nline3\n").unwrap();
    git(repo_path, &["add", "a.txt"]);
    git(repo_path, &["commit", "-m", "initial"]);

    git(repo_path, &["checkout", "-b", "feature"]);
    std::fs::remove_file(repo_path.join("a.txt")).unwrap();
    git(repo_path, &["add", "a.txt"]);
    git(repo_path, &["commit", "-m", "delete a"]);

    let git_repo = perch::git::GitRepo::new(repo_path).unwrap();
    let mb = git_repo.merge_base("main").unwrap().expect("merge base");
    let entries = git_repo.branch_status(mb).unwrap();

    let a = entries
        .iter()
        .find(|e| e.path == "a.txt")
        .expect("a.txt must appear as deleted");
    assert!(matches!(a.status, perch::git::FileStatus::Deleted));
    assert!(
        a.deletions >= 3,
        "deleted file should report deleted-line count; got {}",
        a.deletions
    );
}
