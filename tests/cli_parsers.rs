use git_rt::git::cli::{merge_status_and_numstat, parse_numstat, parse_porcelain_v2};
use git_rt::git::{FileEntry, FileStatus};

#[test]
fn parse_porcelain_v2_untracked_single() {
    let input = b"? untracked.txt\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![("untracked.txt".to_string(), FileStatus::Untracked)]
    );
}

#[test]
fn parse_porcelain_v2_untracked_multiple() {
    let input = b"? a.txt\0? b.txt\0? c.txt\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![
            ("a.txt".to_string(), FileStatus::Untracked),
            ("b.txt".to_string(), FileStatus::Untracked),
            ("c.txt".to_string(), FileStatus::Untracked),
        ]
    );
}

#[test]
fn parse_porcelain_v2_empty_input() {
    let input = b"";
    let result = parse_porcelain_v2(input);
    assert!(result.is_empty());
}

#[test]
fn parse_porcelain_v2_unstaged_modified() {
    let input = b"1 .M N... 100644 100644 100644 abc123 def456 src/main.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![("src/main.rs".to_string(), FileStatus::Modified)]
    );
}

#[test]
fn parse_porcelain_v2_staged_modified() {
    let input = b"1 M. N... 100644 100644 100644 abc123 def456 src/main.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![("src/main.rs".to_string(), FileStatus::Staged)]
    );
}

#[test]
fn parse_porcelain_v2_staged_and_modified() {
    // staged-then-further-modified: should display as Modified
    let input = b"1 MM N... 100644 100644 100644 abc123 def456 src/main.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![("src/main.rs".to_string(), FileStatus::Modified)]
    );
}

#[test]
fn parse_porcelain_v2_staged_added() {
    let input = b"1 A. N... 000000 100644 100644 0000000 abc1234 new.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(result, vec![("new.rs".to_string(), FileStatus::Added)]);
}

#[test]
fn parse_porcelain_v2_deleted_index() {
    let input = b"1 D. N... 100644 000000 000000 abc123 0000000 gone.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(result, vec![("gone.rs".to_string(), FileStatus::Deleted)]);
}

#[test]
fn parse_porcelain_v2_deleted_worktree() {
    let input = b"1 .D N... 100644 100644 000000 abc123 abc123 gone.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(result, vec![("gone.rs".to_string(), FileStatus::Deleted)]);
}

#[test]
fn parse_porcelain_v2_path_with_space() {
    // -z mode: path is raw bytes, no quoting. Spaces are preserved.
    let input = b"1 .M N... 100644 100644 100644 abc123 def456 dir name/file with space.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![(
            "dir name/file with space.rs".to_string(),
            FileStatus::Modified
        )]
    );
}

#[test]
fn parse_porcelain_v2_mixed_with_untracked() {
    let input = b"1 .M N... 100644 100644 100644 abc def src/a.rs\0? src/b.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![
            ("src/a.rs".to_string(), FileStatus::Modified),
            ("src/b.rs".to_string(), FileStatus::Untracked),
        ]
    );
}

#[test]
fn parse_porcelain_v2_rename_emits_delete_then_add() {
    // Rename old.rs → new.rs: emit (old, Deleted) then (new, Added)
    let input = b"2 R. N... 100644 100644 100644 abc def R100 new.rs\0old.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![
            ("old.rs".to_string(), FileStatus::Deleted),
            ("new.rs".to_string(), FileStatus::Added),
        ]
    );
}

#[test]
fn parse_porcelain_v2_unmerged_conflict() {
    let input = b"u UU N... 100644 100644 100644 100644 abc def ghi conflict.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(
        result,
        vec![("conflict.rs".to_string(), FileStatus::Conflicted)]
    );
}

#[test]
fn parse_porcelain_v2_ignored_lines_skipped() {
    // ! lines: should never appear (we don't pass --ignored), but be defensive
    let input = b"! ignored.rs\0? real.rs\0";
    let result = parse_porcelain_v2(input);
    assert_eq!(result, vec![("real.rs".to_string(), FileStatus::Untracked)]);
}

#[test]
fn parse_numstat_single_regular() {
    let input = b"5\t2\tsrc/main.rs\0";
    let result = parse_numstat(input);
    assert_eq!(result, vec![("src/main.rs".to_string(), 5, 2)]);
}

#[test]
fn parse_numstat_multiple() {
    let input = b"5\t2\ta.rs\x00100\t0\tb.rs\0";
    let result = parse_numstat(input);
    assert_eq!(
        result,
        vec![("a.rs".to_string(), 5, 2), ("b.rs".to_string(), 100, 0),]
    );
}

#[test]
fn parse_numstat_binary_file_zeros() {
    let input = b"-\t-\timage.png\0";
    let result = parse_numstat(input);
    assert_eq!(result, vec![("image.png".to_string(), 0, 0)]);
}

#[test]
fn parse_numstat_rename_uses_destination_path() {
    // Rename: `<added>\t<deleted>\t\0<from>\0<to>\0`
    let input = b"3\t1\t\0old/path.rs\0new/path.rs\0";
    let result = parse_numstat(input);
    assert_eq!(result, vec![("new/path.rs".to_string(), 3, 1)]);
}

#[test]
fn parse_numstat_path_with_space() {
    let input = b"7\t3\tdir name/file with space.rs\0";
    let result = parse_numstat(input);
    assert_eq!(
        result,
        vec![("dir name/file with space.rs".to_string(), 7, 3)]
    );
}

#[test]
fn parse_numstat_empty_input() {
    let input = b"";
    let result = parse_numstat(input);
    assert!(result.is_empty());
}

#[test]
fn merge_attaches_numstat_to_status_entries() {
    let status = vec![
        ("a.rs".to_string(), FileStatus::Modified),
        ("b.rs".to_string(), FileStatus::Added),
    ];
    let numstat = vec![("a.rs".to_string(), 5, 2), ("b.rs".to_string(), 10, 0)];
    let tmp = tempfile::tempdir().unwrap();
    let result: Vec<FileEntry> = merge_status_and_numstat(status, numstat, tmp.path(), None);

    assert_eq!(result.len(), 2);
    let a = result.iter().find(|e| e.path == "a.rs").unwrap();
    assert_eq!(a.insertions, 5);
    assert_eq!(a.deletions, 2);
    assert!(matches!(a.status, FileStatus::Modified));
    let b = result.iter().find(|e| e.path == "b.rs").unwrap();
    assert_eq!(b.insertions, 10);
    assert_eq!(b.deletions, 0);
    assert!(matches!(b.status, FileStatus::Added));
}

#[test]
fn merge_sorts_output_by_path() {
    let status = vec![
        ("z.rs".to_string(), FileStatus::Modified),
        ("a.rs".to_string(), FileStatus::Modified),
        ("m.rs".to_string(), FileStatus::Modified),
    ];
    let numstat = vec![];
    let tmp = tempfile::tempdir().unwrap();
    let result = merge_status_and_numstat(status, numstat, tmp.path(), None);

    let paths: Vec<&str> = result.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["a.rs", "m.rs", "z.rs"]);
}

#[test]
fn merge_untracked_file_counts_lines_from_disk() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("new.txt"), "line1\nline2\nline3\n").unwrap();

    let status = vec![("new.txt".to_string(), FileStatus::Untracked)];
    let numstat = vec![];
    let result = merge_status_and_numstat(status, numstat, tmp.path(), None);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, "new.txt");
    assert_eq!(result[0].insertions, 3);
    assert_eq!(result[0].deletions, 0);
    assert!(matches!(result[0].status, FileStatus::Untracked));
}

#[test]
fn merge_untracked_unreadable_file_reports_zero() {
    // Path that doesn't exist on disk: read fails; treat as 0/0.
    let status = vec![("ghost.txt".to_string(), FileStatus::Untracked)];
    let numstat = vec![];
    let tmp = tempfile::tempdir().unwrap();
    let result = merge_status_and_numstat(status, numstat, tmp.path(), None);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].insertions, 0);
    assert_eq!(result[0].deletions, 0);
}
