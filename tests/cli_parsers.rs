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
