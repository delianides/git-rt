use std::path::Path;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::git::{FileEntry, FileStatus};

/// A parsed segment of a format string
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatSegment {
    /// A token like %s, %f, %+, %-, etc.
    Token(char),
    /// Literal text between tokens
    Literal(String),
    /// Right-align marker (%=) — everything after this is right-aligned
    RightAlign,
}

/// Parse a vim-style format string into segments
pub fn parse_format(fmt: &str) -> Vec<FormatSegment> {
    let mut segments = Vec::new();
    let mut chars = fmt.chars().peekable();
    let mut literal = String::new();
    let mut seen_right_align = false;

    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek() {
                Some('%') => {
                    chars.next();
                    literal.push('%');
                }
                Some(&'=') => {
                    chars.next();
                    if !seen_right_align {
                        if !literal.is_empty() {
                            segments.push(FormatSegment::Literal(literal.clone()));
                            literal.clear();
                        }
                        segments.push(FormatSegment::RightAlign);
                        seen_right_align = true;
                    } else {
                        literal.push_str("%=");
                    }
                }
                Some(&token) => {
                    if !literal.is_empty() {
                        segments.push(FormatSegment::Literal(literal.clone()));
                        literal.clear();
                    }
                    chars.next();
                    segments.push(FormatSegment::Token(token));
                }
                None => {
                    literal.push('%');
                }
            }
        } else {
            literal.push(ch);
        }
    }

    if !literal.is_empty() {
        segments.push(FormatSegment::Literal(literal));
    }

    segments
}

const GRAPH_WIDTH: usize = 20;

/// Resolve a single format token to its string value for a given file entry
pub fn resolve_token(token: char, entry: &FileEntry, branch: &str) -> String {
    match token {
        's' => status_char(&entry.status).to_string(),
        'S' => staged_char(&entry.status).to_string(),
        'f' => entry.path.clone(),
        'n' => Path::new(&entry.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        'd' => Path::new(&entry.path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        'e' => Path::new(&entry.path)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default(),
        '-' => format!("-{}", entry.deletions),
        '+' => format!("+{}", entry.insertions),
        't' => (entry.insertions + entry.deletions).to_string(),
        'g' => render_change_graph(entry.insertions, entry.deletions),
        'b' => branch.to_string(),
        _ => format!("%{token}"),
    }
}

fn status_char(status: &FileStatus) -> char {
    match status {
        FileStatus::Modified => 'M',
        FileStatus::Added => 'A',
        FileStatus::Deleted => 'D',
        FileStatus::Renamed => 'R',
        FileStatus::Untracked => '?',
        FileStatus::Staged => 'S',
        FileStatus::Conflicted => 'C',
    }
}

fn staged_char(status: &FileStatus) -> char {
    match status {
        FileStatus::Staged => 'S',
        FileStatus::Added => 'A',
        _ => ' ',
    }
}

fn render_change_graph(insertions: usize, deletions: usize) -> String {
    let total = insertions + deletions;
    if total == 0 {
        return String::new();
    }

    let width = GRAPH_WIDTH.min(total);
    let ins_width = ((insertions as f64 / total as f64) * width as f64).round() as usize;
    let del_width = width - ins_width;

    let mut graph = String::with_capacity(width);
    for _ in 0..ins_width {
        graph.push('+');
    }
    for _ in 0..del_width {
        graph.push('-');
    }
    graph
}

/// Compute the max width for each token column across all file entries.
/// Returns a Vec with one width per Token segment that appears before any
/// `RightAlign` marker. Tokens after the marker are not padded.
pub fn compute_column_widths(
    segments: &[FormatSegment],
    entries: &[FileEntry],
    branch: &str,
) -> Vec<usize> {
    // Only count tokens before RightAlign
    let token_count = segments
        .iter()
        .take_while(|s| !matches!(s, FormatSegment::RightAlign))
        .filter(|s| matches!(s, FormatSegment::Token(_)))
        .count();

    let mut widths = vec![0usize; token_count];

    for entry in entries {
        let mut token_idx = 0;
        for segment in segments {
            if matches!(segment, FormatSegment::RightAlign) {
                break;
            }
            if let FormatSegment::Token(ch) = segment {
                let value = resolve_token(*ch, entry, branch);
                widths[token_idx] = widths[token_idx].max(value.len());
                token_idx += 1;
            }
        }
    }

    widths
}

/// Get the color for a format token
fn token_color(token: char, entry: &FileEntry) -> Option<Color> {
    match token {
        's' => Some(status_color(&entry.status)),
        'S' => Some(status_color(&entry.status)),
        '-' => Some(Color::Red),
        '+' => Some(Color::Green),
        'g' => None, // graph has mixed colors, handled separately
        _ => None,
    }
}

fn status_color(status: &FileStatus) -> Color {
    match status {
        FileStatus::Modified => Color::Yellow,
        FileStatus::Added | FileStatus::Untracked => Color::Green,
        FileStatus::Deleted => Color::Red,
        FileStatus::Renamed => Color::Cyan,
        FileStatus::Staged => Color::Green,
        FileStatus::Conflicted => Color::Magenta,
    }
}

/// Render a file line as a styled ratatui Line with colored spans
pub fn render_file_line<'a>(
    segments: &[FormatSegment],
    entry: &FileEntry,
    branch: &str,
    widths: &[usize],
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut token_idx = 0;

    for segment in segments {
        match segment {
            FormatSegment::Literal(text) => {
                spans.push(Span::raw(text.to_string()));
            }
            FormatSegment::Token(ch) => {
                let value = resolve_token(*ch, entry, branch);
                let width = widths.get(token_idx).copied().unwrap_or(value.len());

                if *ch == 'g' {
                    // Change graph gets split into colored + and - spans
                    let ins_count = value.chars().take_while(|c| *c == '+').count();
                    let (ins_part, del_part) = value.split_at(ins_count);
                    let pad_needed = width.saturating_sub(value.len());
                    spans.push(Span::styled(
                        ins_part.to_string(),
                        Style::default().fg(Color::Green),
                    ));
                    spans.push(Span::styled(
                        format!("{}{}", del_part, " ".repeat(pad_needed)),
                        Style::default().fg(Color::Red),
                    ));
                } else {
                    let padded = format!("{:<width$}", value);
                    let style = match token_color(*ch, entry) {
                        Some(color) => Style::default().fg(color),
                        None => Style::default(),
                    };
                    spans.push(Span::styled(padded, style));
                }
                token_idx += 1;
            }
            FormatSegment::RightAlign => {
                // No-op for now; right-align rendering handled in a later task
            }
        }
    }

    Line::from(spans)
}

/// Render a file line as a plain string (for testing). Pads token columns to given widths.
pub fn render_file_line_plain(
    segments: &[FormatSegment],
    entry: &FileEntry,
    branch: &str,
    widths: &[usize],
) -> String {
    let mut result = String::new();
    let mut token_idx = 0;

    for segment in segments {
        match segment {
            FormatSegment::Literal(text) => {
                result.push_str(text);
            }
            FormatSegment::Token(ch) => {
                let value = resolve_token(*ch, entry, branch);
                let width = widths.get(token_idx).copied().unwrap_or(value.len());
                result.push_str(&format!("{:<width$}", value));
                token_idx += 1;
            }
            FormatSegment::RightAlign => {
                // No-op for now; right-align rendering handled in a later task
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_default_format() {
        let segments = parse_format("%s %f %- %+");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Token('s'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('f'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('-'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('+'),
            ]
        );
    }

    #[test]
    fn test_parse_literal_only() {
        let segments = parse_format("hello world");
        assert_eq!(
            segments,
            vec![FormatSegment::Literal("hello world".to_string())]
        );
    }

    #[test]
    fn test_parse_escaped_percent() {
        let segments = parse_format("%%done");
        assert_eq!(segments, vec![FormatSegment::Literal("%done".to_string())]);
    }

    #[test]
    fn test_parse_adjacent_tokens() {
        let segments = parse_format("%s%f");
        assert_eq!(
            segments,
            vec![FormatSegment::Token('s'), FormatSegment::Token('f'),]
        );
    }

    #[test]
    fn test_parse_tokens_with_surrounding_literal() {
        let segments = parse_format("[%s] %f | %- %+");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Literal("[".to_string()),
                FormatSegment::Token('s'),
                FormatSegment::Literal("] ".to_string()),
                FormatSegment::Token('f'),
                FormatSegment::Literal(" | ".to_string()),
                FormatSegment::Token('-'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('+'),
            ]
        );
    }

    #[test]
    fn test_parse_empty_string() {
        let segments = parse_format("");
        assert!(segments.is_empty());
    }

    use crate::git::{FileEntry, FileStatus};

    fn test_entry() -> FileEntry {
        FileEntry {
            path: "src/config/mod.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 12,
            deletions: 3,
        }
    }

    #[test]
    fn test_resolve_status_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('s', &entry, "main"), "M");
    }

    #[test]
    fn test_resolve_path_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('f', &entry, "main"), "src/config/mod.rs");
    }

    #[test]
    fn test_resolve_filename_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('n', &entry, "main"), "mod.rs");
    }

    #[test]
    fn test_resolve_directory_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('d', &entry, "main"), "src/config");
    }

    #[test]
    fn test_resolve_extension_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('e', &entry, "main"), "rs");
    }

    #[test]
    fn test_resolve_insertions_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('+', &entry, "main"), "+12");
    }

    #[test]
    fn test_resolve_deletions_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('-', &entry, "main"), "-3");
    }

    #[test]
    fn test_resolve_total_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('t', &entry, "main"), "15");
    }

    #[test]
    fn test_resolve_branch_token() {
        let entry = test_entry();
        assert_eq!(resolve_token('b', &entry, "main"), "main");
    }

    #[test]
    fn test_resolve_change_graph_token() {
        let entry = test_entry();
        let graph = resolve_token('g', &entry, "main");
        assert!(graph.contains('+'));
        assert!(graph.contains('-'));
    }

    #[test]
    fn test_resolve_no_extension() {
        let entry = FileEntry {
            path: "Makefile".to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 0,
        };
        assert_eq!(resolve_token('e', &entry, "main"), "");
    }

    #[test]
    fn test_resolve_no_directory() {
        let entry = FileEntry {
            path: "README.md".to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 0,
        };
        assert_eq!(resolve_token('d', &entry, "main"), "");
    }

    #[test]
    fn test_change_graph_zero_changes() {
        let entry = FileEntry {
            path: "empty.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 0,
            deletions: 0,
        };
        assert_eq!(resolve_token('g', &entry, "main"), "");
    }

    #[test]
    fn test_change_graph_all_insertions() {
        let entry = FileEntry {
            path: "new.rs".to_string(),
            status: FileStatus::Added,
            insertions: 10,
            deletions: 0,
        };
        let graph = resolve_token('g', &entry, "main");
        assert!(!graph.contains('-'));
        assert!(graph.contains('+'));
    }

    #[test]
    fn test_change_graph_all_deletions() {
        let entry = FileEntry {
            path: "old.rs".to_string(),
            status: FileStatus::Deleted,
            insertions: 0,
            deletions: 10,
        };
        let graph = resolve_token('g', &entry, "main");
        assert!(graph.contains('-'));
        assert!(!graph.contains('+'));
    }

    #[test]
    fn test_compute_column_widths() {
        let segments = parse_format("%s %f");
        let entries = vec![
            FileEntry {
                path: "short.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 1,
                deletions: 0,
            },
            FileEntry {
                path: "very/long/path/file.rs".to_string(),
                status: FileStatus::Added,
                insertions: 100,
                deletions: 50,
            },
        ];
        let widths = compute_column_widths(&segments, &entries, "main");
        // Token 's' -> max of "M" and "A" = 1
        // Token 'f' -> max of "short.rs" (8) and "very/long/path/file.rs" (22) = 22
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0], 1);
        assert_eq!(widths[1], 22);
    }

    #[test]
    fn test_compute_column_widths_with_numbers() {
        let segments = parse_format("%- %+");
        let entries = vec![
            FileEntry {
                path: "a.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 1,
                deletions: 100,
            },
            FileEntry {
                path: "b.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 1000,
                deletions: 2,
            },
        ];
        let widths = compute_column_widths(&segments, &entries, "main");
        // '-' -> max of "-100" (4) and "-2" (2) = 4
        // '+' -> max of "+1" (2) and "+1000" (5) = 5
        assert_eq!(widths[0], 4);
        assert_eq!(widths[1], 5);
    }

    use ratatui::style::Color;

    #[test]
    fn test_render_file_line_styled_has_correct_span_count() {
        let segments = parse_format("%s %f %- %+");
        let entry = FileEntry {
            path: "main.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 5,
            deletions: 2,
        };
        let widths = vec![1, 7, 2, 2];
        let line = render_file_line(&segments, &entry, "main", &widths);
        // Expect spans: status, literal " ", path, literal " ", deletions, literal " ", insertions
        assert_eq!(line.spans.len(), 7);
    }

    #[test]
    fn test_render_file_line_styled_colors() {
        let segments = parse_format("%- %+");
        let entry = FileEntry {
            path: "a.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 5,
            deletions: 2,
        };
        let widths = vec![2, 2];
        let line = render_file_line(&segments, &entry, "main", &widths);
        // First span is deletions (red), then literal, then insertions (green)
        assert_eq!(line.spans[0].style.fg, Some(Color::Red));
        assert_eq!(line.spans[2].style.fg, Some(Color::Green));
    }

    #[test]
    fn test_render_file_line_plain_text() {
        let segments = parse_format("%s %f %- %+");
        let entry = FileEntry {
            path: "main.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 5,
            deletions: 2,
        };
        let widths = vec![1, 7, 2, 2];
        let text = render_file_line_plain(&segments, &entry, "main", &widths);
        assert_eq!(text, "M main.rs -2 +5");
    }

    #[test]
    fn test_parse_right_align_marker() {
        let segments = parse_format("%s %f %= %- %+");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Token('s'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('f'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::RightAlign,
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('-'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('+'),
            ]
        );
    }

    #[test]
    fn test_parse_multiple_right_align_only_first() {
        let segments = parse_format("%f %= %- %= %+");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Token('f'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::RightAlign,
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('-'),
                FormatSegment::Literal(" %= ".to_string()),
                FormatSegment::Token('+'),
            ]
        );
    }

    #[test]
    fn test_parse_right_align_at_start() {
        let segments = parse_format("%= %f");
        assert_eq!(
            segments,
            vec![
                FormatSegment::RightAlign,
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('f'),
            ]
        );
    }

    #[test]
    fn test_compute_column_widths_stops_at_right_align() {
        let segments = parse_format("%s %f %= %- %+");
        let entries = vec![
            FileEntry {
                path: "short.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 1,
                deletions: 0,
            },
            FileEntry {
                path: "very/long/path/file.rs".to_string(),
                status: FileStatus::Added,
                insertions: 100,
                deletions: 50,
            },
        ];
        let widths = compute_column_widths(&segments, &entries, "main");
        // Only tokens before %= are padded: 's' and 'f'
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0], 1);
        assert_eq!(widths[1], 22);
    }
}
