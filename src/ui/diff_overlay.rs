//! Full-screen diff overlay.
//!
//! Rendered when the user activates diff view (Enter / d / Space / l / Right)
//! on a selected file. Displays a centred 85% panel with line-numbered,
//! coloured diff output. Scrollable with j / k / ↑ / ↓. Dismissible with
//! Esc / q / d / Space / h / ←.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::git::{DiffLineKind, FileDiff};
use crate::ui::colors;

/// Parse a hunk header like `@@ -14,10 +14,2 @@` or `@@ -1 +1 @@`
/// and return `(old_start, new_start)`.
pub fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    let trimmed = header.trim();
    if !trimmed.starts_with("@@") {
        return None;
    }

    // Find the range part between the @@ markers
    let inner = trimmed.strip_prefix("@@")?.trim_start();
    // inner looks like: "-14,10 +14,2 @@ optional context"
    // or "-1 +1 @@ optional context"

    let parts: Vec<&str> = inner.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return None;
    }

    let old_part = parts[0]; // "-14,10" or "-1"
    let new_part = parts[1]; // "+14,2" or "+1"

    let old_start = old_part
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()?;
    let new_start = new_part
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()?;

    Some((old_start, new_start))
}

/// Build the line-number gutter string for one diff line.
///
/// Both columns are right-aligned to `w` and separated by a single space, so
/// the old and new numbers never collide regardless of magnitude. A trailing
/// space separates the gutter from the line content. Additions blank the old
/// column; deletions blank the new column; hunk headers are blank across the
/// full `2 * w + 2` gutter width.
fn format_gutter(kind: &DiffLineKind, old_line: usize, new_line: usize, w: usize) -> String {
    match kind {
        DiffLineKind::Addition => format!("{:>w$} {:>w$} ", "", new_line, w = w),
        DiffLineKind::Deletion => format!("{:>w$} {:>w$} ", old_line, "", w = w),
        DiffLineKind::Context => format!("{:>w$} {:>w$} ", old_line, new_line, w = w),
        DiffLineKind::HunkHeader => " ".repeat(2 * w + 2),
    }
}

/// Per-column digit width for a file's diff gutter: the digit count of the
/// largest line number that will be displayed across both the old and new
/// columns. Floored at 1 so empty/degenerate diffs still render a stable
/// gutter. Sizing per-file keeps narrow files compact while guaranteeing
/// 5+ digit line numbers never collide.
fn gutter_width(diff: &FileDiff) -> usize {
    let mut max_line = 0usize;
    for hunk in &diff.hunks {
        let (mut old_line, mut new_line) = parse_hunk_header(&hunk.header).unwrap_or((1, 1));
        for diff_line in &hunk.lines {
            match diff_line.kind {
                DiffLineKind::Addition => {
                    max_line = max_line.max(new_line);
                    new_line += 1;
                }
                DiffLineKind::Deletion => {
                    max_line = max_line.max(old_line);
                    old_line += 1;
                }
                DiffLineKind::Context => {
                    max_line = max_line.max(old_line).max(new_line);
                    old_line += 1;
                    new_line += 1;
                }
                DiffLineKind::HunkHeader => {}
            }
        }
    }
    max_line.to_string().len().max(1)
}

/// Render the diff overlay onto `frame`.
///
/// The overlay is drawn as a centred, bordered panel on top of whatever is
/// already rendered. The caller should render the main pane first, then call
/// this function.
pub fn render_diff_overlay(
    frame: &mut Frame,
    diff: &FileDiff,
    file_path: &str,
    insertions: usize,
    deletions: usize,
    scroll: usize,
) {
    let area = frame.area();
    let overlay_rect = centered_rect(85, 85, area);

    // Clear the background behind the overlay
    frame.render_widget(Clear, overlay_rect);

    // Build the title
    let title = format!(" {} +{} -{} ", file_path, insertions, deletions);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::BORDER_FOCUSED))
        .title(title)
        .title_style(Style::default().fg(colors::HEADER_TEXT));

    let inner = block.inner(overlay_rect);
    frame.render_widget(block, overlay_rect);

    // Build diff lines with line numbers and colours
    let mut lines: Vec<Line> = Vec::new();

    for hunk in &diff.hunks {
        // Hunk header line
        lines.push(Line::from(vec![
            Span::styled("         ", Style::default().fg(colors::DIFF_LINE_NUMBER)),
            Span::styled(
                &hunk.header,
                Style::default()
                    .fg(colors::DIFF_HUNK_HEADER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let (mut old_line, mut new_line) = parse_hunk_header(&hunk.header).unwrap_or((1, 1));

        for diff_line in &hunk.lines {
            let (gutter, style) = match diff_line.kind {
                DiffLineKind::Addition => {
                    let g = format!("{:>4}{:>4} ", "    ", new_line);
                    new_line += 1;
                    (g, Style::default().fg(colors::DIFF_ADD_FG))
                }
                DiffLineKind::Deletion => {
                    let g = format!("{:>4}{:>4} ", old_line, "    ");
                    old_line += 1;
                    (g, Style::default().fg(colors::DIFF_DEL_FG))
                }
                DiffLineKind::Context => {
                    let g = format!("{:>4}{:>4} ", old_line, new_line);
                    old_line += 1;
                    new_line += 1;
                    (g, Style::default().fg(colors::DIFF_CONTEXT))
                }
                DiffLineKind::HunkHeader => (
                    "         ".to_string(),
                    Style::default().fg(colors::DIFF_HUNK_HEADER),
                ),
            };

            let prefix = match diff_line.kind {
                DiffLineKind::Addition => "+",
                DiffLineKind::Deletion => "-",
                DiffLineKind::Context => " ",
                DiffLineKind::HunkHeader => "@",
            };

            lines.push(Line::from(vec![
                Span::styled(gutter, Style::default().fg(colors::DIFF_LINE_NUMBER)),
                Span::styled(format!("{prefix} {}", diff_line.content), style),
            ]));
        }
    }

    // Apply scroll offset
    let visible_lines: Vec<Line> = lines.into_iter().skip(scroll).collect();

    let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// Height in rows of the diff overlay's scrollable inner area for a given
/// frame `area`. Mirrors `render_diff_overlay`'s geometry: the centred 85%
/// panel minus its top and bottom border rows.
///
/// This is a row count, not a logical-line count — diff lines that wrap span
/// multiple rows, so page scrolling sized by this value can overshoot
/// visually when lines wrap. See `AppState::diff_total_lines`.
pub fn inner_height(area: Rect) -> usize {
    let panel = centered_rect(85, 85, area);
    panel.height.saturating_sub(2) as usize
}

/// Return a centred `Rect` that occupies `percent_x`% width and `percent_y`%
/// height of `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    let h = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header_standard() {
        assert_eq!(parse_hunk_header("@@ -14,10 +14,2 @@"), Some((14, 14)));
    }

    #[test]
    fn test_parse_hunk_header_single_line() {
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1)));
    }

    #[test]
    fn test_parse_hunk_header_with_context() {
        assert_eq!(
            parse_hunk_header("@@ -10,3 +10,4 @@ fn main()"),
            Some((10, 10))
        );
    }

    #[test]
    fn test_parse_hunk_header_invalid() {
        assert_eq!(parse_hunk_header("not a header"), None);
    }

    #[test]
    fn test_inner_height_subtracts_border() {
        // 100-tall area -> 85% = 85 rows for the panel; minus top+bottom border = 83.
        let area = Rect::new(0, 0, 100, 100);
        assert_eq!(inner_height(area), 83);
    }

    #[test]
    fn test_format_gutter_context_separates_columns() {
        // width 2, both columns present
        let g = format_gutter(&DiffLineKind::Context, 12, 34, 2);
        assert_eq!(g, "12 34 ");
    }

    #[test]
    fn test_format_gutter_addition_blanks_old_column() {
        let g = format_gutter(&DiffLineKind::Addition, 12, 34, 2);
        assert_eq!(g, "   34 ");
    }

    #[test]
    fn test_format_gutter_deletion_blanks_new_column() {
        let g = format_gutter(&DiffLineKind::Deletion, 12, 34, 2);
        assert_eq!(g, "12    ");
    }

    #[test]
    fn test_format_gutter_five_digits_stay_separated() {
        // The bug: 5-digit numbers used to collide. With width 5 they must
        // be separated by at least one space.
        let g = format_gutter(&DiffLineKind::Context, 12345, 12346, 5);
        assert_eq!(g, "12345 12346 ");
        assert!(g.contains("12345 12346"));
    }

    #[test]
    fn test_format_gutter_hunk_header_is_blank_of_matching_width() {
        // Blank gutter must span 2*w + 2 columns to align with number rows.
        let g = format_gutter(&DiffLineKind::HunkHeader, 0, 0, 4);
        assert_eq!(g, " ".repeat(4 * 2 + 2));
        assert_eq!(g.len(), 10);
    }

    fn diff_with(hunks: Vec<(&str, Vec<(DiffLineKind, &str)>)>) -> FileDiff {
        use crate::git::{DiffHunk, DiffLine};
        FileDiff {
            hunks: hunks
                .into_iter()
                .map(|(header, lines)| DiffHunk {
                    header: header.to_string(),
                    lines: lines
                        .into_iter()
                        .map(|(kind, content)| DiffLine {
                            kind,
                            content: content.to_string(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn test_gutter_width_empty_diff_is_one() {
        let diff = FileDiff::default();
        assert_eq!(gutter_width(&diff), 1);
    }

    #[test]
    fn test_gutter_width_small_numbers() {
        // Lines run 1..=3 on each side -> 1 digit.
        let diff = diff_with(vec![(
            "@@ -1,3 +1,3 @@",
            vec![
                (DiffLineKind::Context, "a"),
                (DiffLineKind::Context, "b"),
                (DiffLineKind::Context, "c"),
            ],
        )]);
        assert_eq!(gutter_width(&diff), 1);
    }

    #[test]
    fn test_gutter_width_tracks_largest_across_hunks() {
        // Second hunk starts at 9998 with 3 context lines -> reaches 10000 (5 digits).
        let diff = diff_with(vec![
            (
                "@@ -1,2 +1,2 @@",
                vec![(DiffLineKind::Context, "a"), (DiffLineKind::Context, "b")],
            ),
            (
                "@@ -9998,3 +9998,3 @@",
                vec![
                    (DiffLineKind::Context, "x"),
                    (DiffLineKind::Context, "y"),
                    (DiffLineKind::Context, "z"),
                ],
            ),
        ]);
        assert_eq!(gutter_width(&diff), 5);
    }
}
