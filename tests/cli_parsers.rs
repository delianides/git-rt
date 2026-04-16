use git_rt::git::cli::parse_porcelain_v2;
use git_rt::git::FileStatus;

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
