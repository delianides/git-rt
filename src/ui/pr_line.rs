//! Compact PR status line rendered directly below the main pane border
//! when a PR is open against the current branch.
//!
//! Format:
//!
//! ```text
//!  PR #142  ✓ clean  ✓ 12  ✗ 3
//! ```
//!
//! Segments, in order:
//! - `PR #<number>` — in the PR-state color (green/red/magenta/gray)
//! - mergeable indicator: `✓ clean`, `✗ conflicts`, `⚠ behind`, or omitted
//!   when the status is still `Unknown`
//! - check counts: `✓ <passed>` (green) and `✗ <failed>` (red). Segments
//!   with a zero count are omitted. If the PR has no checks at all, the
//!   whole checks segment is omitted.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::state::{AppState, MergeableStatus, PrDisplayInfo, PrState, PrStatus};
use crate::theme::Theme;

/// Should the PR line be rendered at all? True only when PR data has been
/// loaded (not loading, not errored, not missing).
pub fn has_pr_line(state: &AppState) -> bool {
    state.pr_state().info.is_some()
}

/// PR-state color mapping (shared with the main-pane border indicator).
pub fn pr_state_color(status: &PrStatus) -> Color {
    match status {
        PrStatus::Open => Color::Green,
        PrStatus::Closed => Color::Red,
        PrStatus::Merged => Color::Magenta,
        PrStatus::Draft => Color::Gray,
    }
}

/// Build the styled `Line` for the PR status row.
///
/// Returns `None` when there is no PR data to render, in which case the
/// caller should skip rendering the row entirely.
pub fn build_pr_line(pr_state: &PrState, theme: &Theme) -> Option<Line<'static>> {
    let info = pr_state.info.as_ref()?;
    Some(build_line_from_info(info, theme))
}

fn build_line_from_info(info: &PrDisplayInfo, theme: &Theme) -> Line<'static> {
    let state_color = pr_state_color(&info.state);
    let sep_style = Style::default().fg(theme.header_separator);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(" "));

    // PR number in state color + bold.
    // OSC 8 hyperlink is applied post-render in render_pr_line() as a buffer
    // workaround for https://github.com/ratatui/ratatui/issues/902.
    spans.push(Span::styled(
        format!("PR #{}", info.number),
        Style::default()
            .fg(state_color)
            .add_modifier(Modifier::BOLD),
    ));

    // Mergeable indicator.
    if let Some((icon, label, color)) = mergeable_indicator(&info.mergeable) {
        spans.push(Span::styled("  ", sep_style));
        spans.push(Span::styled(icon.to_string(), Style::default().fg(color)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label.to_string(), Style::default().fg(color)));
    }

    // Check counts — hybrid display:
    // - failures present → per-category breakdown (passed/failed/pending)
    // - pending, no failures → yellow fraction (completed/total)
    // - all passed → green fraction (passed/total)
    let checks = &info.checks;
    if checks.total > 0 {
        if checks.failed > 0 {
            // Failure mode: per-category breakdown
            if checks.passed > 0 {
                spans.push(Span::styled("  ", sep_style));
                spans.push(Span::styled("✓ ", Style::default().fg(Color::Green)));
                spans.push(Span::styled(
                    checks.passed.to_string(),
                    Style::default().fg(Color::Green),
                ));
            }
            spans.push(Span::styled("  ", sep_style));
            spans.push(Span::styled("✗ ", Style::default().fg(Color::Red)));
            spans.push(Span::styled(
                checks.failed.to_string(),
                Style::default().fg(Color::Red),
            ));
            if checks.pending > 0 {
                spans.push(Span::styled("  ", sep_style));
                spans.push(Span::styled("◐ ", Style::default().fg(Color::Yellow)));
                spans.push(Span::styled(
                    checks.pending.to_string(),
                    Style::default().fg(Color::Yellow),
                ));
            }
        } else if checks.pending > 0 {
            // In progress: yellow fraction
            let completed = checks.passed + checks.skipped;
            spans.push(Span::styled("  ", sep_style));
            spans.push(Span::styled("◐ ", Style::default().fg(Color::Yellow)));
            spans.push(Span::styled(
                format!("{}/{}", completed, checks.total),
                Style::default().fg(Color::Yellow),
            ));
        } else {
            // All passed: green fraction
            spans.push(Span::styled("  ", sep_style));
            spans.push(Span::styled("✓ ", Style::default().fg(Color::Green)));
            spans.push(Span::styled(
                format!("{}/{}", checks.passed, checks.total),
                Style::default().fg(Color::Green),
            ));
        }
    }

    Line::from(spans)
}

/// Mergeable status → (icon, label, color). `None` when the status is
/// `Unknown` (still resolving) — we simply omit the segment.
fn mergeable_indicator(status: &MergeableStatus) -> Option<(&'static str, &'static str, Color)> {
    match status {
        MergeableStatus::Clean => Some(("✓", "clean", Color::Green)),
        MergeableStatus::Conflicts => Some(("✗", "conflicts", Color::Red)),
        MergeableStatus::Behind => Some(("⚠", "behind", Color::Yellow)),
        MergeableStatus::Unknown => None,
    }
}

/// Render the PR status line into `area`. Expects a 1-row Rect.
///
/// After rendering the line normally via ratatui, this applies an OSC 8
/// hyperlink to the PR number cells in the buffer. This is a workaround
/// for <https://github.com/ratatui/ratatui/issues/902> — ratatui
/// miscalculates the width of ANSI escape sequences, so we render plain
/// text first and patch the buffer cells with hyperlink escapes afterward.
pub fn render_pr_line(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let pr_state = state.pr_state();
    if let Some(line) = build_pr_line(pr_state, theme) {
        frame.render_widget(Paragraph::new(line), area);

        // Apply OSC 8 hyperlink to the PR number cells in the buffer.
        if let Some(info) = &pr_state.info {
            if !info.url.is_empty() {
                let pr_label = format!("PR #{}", info.number);
                // The PR number starts at column 1 (column 0 is leading space).
                let start_col = area.x + 1;
                let buf = frame.buffer_mut();
                for (i, ch) in pr_label.chars().enumerate() {
                    let col = start_col + i as u16;
                    if col < area.x + area.width {
                        let hyperlink = format!("\x1b]8;;{}\x07{}\x1b]8;;\x07", info.url, ch);
                        buf[(col, area.y)].set_symbol(&hyperlink);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CheckInfo, CheckStatus, ChecksInfo};
    use std::time::Duration;

    /// Strip styling from a `Line` and return its raw concatenated text.
    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    fn test_theme() -> Theme {
        crate::theme::load_theme(crate::theme::DEFAULT_THEME_NAME, None)
    }

    fn make_info(
        mergeable: MergeableStatus,
        passed: usize,
        failed: usize,
        pending: usize,
    ) -> PrDisplayInfo {
        PrDisplayInfo {
            number: 142,
            title: "t".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: passed + failed + pending,
                passed,
                failed,
                pending,
                skipped: 0,
                checks: (0..(passed + failed + pending))
                    .map(|i| CheckInfo {
                        name: format!("check-{i}"),
                        status: if i < passed {
                            CheckStatus::Passed
                        } else if i < passed + failed {
                            CheckStatus::Failed
                        } else {
                            CheckStatus::Pending
                        },
                    })
                    .collect(),
            },
            comment_count: 0,
            mergeable,
            labels: vec![],
            assignees: vec![],
            url: String::new(),
        }
    }

    fn state_with(info: PrDisplayInfo) -> AppState {
        let mut s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        s.set_pr_info(info);
        s
    }

    #[test]
    fn test_has_pr_line_false_without_info() {
        let s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!has_pr_line(&s));
    }

    #[test]
    fn test_has_pr_line_true_with_info() {
        let s = state_with(make_info(MergeableStatus::Clean, 12, 0, 0));
        assert!(has_pr_line(&s));
    }

    #[test]
    fn test_build_pr_line_clean_with_passing_checks() {
        let info = make_info(MergeableStatus::Clean, 12, 0, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean  ✓ 12/12");
    }

    #[test]
    fn test_build_pr_line_clean_with_mixed_checks() {
        let info = make_info(MergeableStatus::Clean, 12, 3, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean  ✓ 12  ✗ 3");
    }

    #[test]
    fn test_build_pr_line_conflicts_with_failing_checks() {
        let info = make_info(MergeableStatus::Conflicts, 0, 5, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✗ conflicts  ✗ 5");
    }

    #[test]
    fn test_build_pr_line_no_checks() {
        let info = make_info(MergeableStatus::Clean, 0, 0, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean");
    }

    #[test]
    fn test_build_pr_line_unknown_mergeable_omits_segment() {
        let info = make_info(MergeableStatus::Unknown, 12, 0, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ 12/12");
    }

    #[test]
    fn test_build_pr_line_behind_base() {
        let info = make_info(MergeableStatus::Behind, 12, 0, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ⚠ behind  ✓ 12/12");
    }

    #[test]
    fn test_build_pr_line_returns_none_without_info() {
        let pr = PrState::default();
        assert!(build_pr_line(&pr, &test_theme()).is_none());
    }

    #[test]
    fn test_pr_number_is_plain_text_in_line() {
        // OSC 8 hyperlinks are applied at the buffer level in render_pr_line(),
        // not embedded in span content. Verify the line contains plain text.
        let mut info = make_info(MergeableStatus::Clean, 0, 0, 0);
        info.url = "https://github.com/owner/repo/pull/142".to_string();
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        let text = line_text(&line);
        assert!(text.contains("PR #142"), "missing PR number");
        assert!(
            !text.contains("\x1b]8;;"),
            "OSC 8 should not be in span content"
        );
    }

    #[test]
    fn test_checks_all_pending_shows_fraction() {
        let info = make_info(MergeableStatus::Clean, 0, 0, 12);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean  ◐ 0/12");
    }

    #[test]
    fn test_checks_mixed_pending_no_failures_shows_fraction() {
        let info = make_info(MergeableStatus::Clean, 8, 0, 4);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean  ◐ 8/12");
    }

    #[test]
    fn test_checks_with_failures_shows_per_category() {
        let info = make_info(MergeableStatus::Clean, 9, 2, 1);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean  ✓ 9  ✗ 2  ◐ 1");
    }

    #[test]
    fn test_checks_all_passed_shows_green_fraction() {
        let info = make_info(MergeableStatus::Clean, 12, 0, 0);
        let line = build_pr_line(&pr_state_with(info), &test_theme()).unwrap();
        assert_eq!(line_text(&line), " PR #142  ✓ clean  ✓ 12/12");
    }

    /// Helper: make a `PrState` containing the given display info.
    fn pr_state_with(info: PrDisplayInfo) -> PrState {
        PrState {
            info: Some(info),
            error: None,
            loading: false,
        }
    }
}
