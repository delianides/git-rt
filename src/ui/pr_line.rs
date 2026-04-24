//! Compact PR status line rendered in the 1-row bottom bar when a PR is
//! open against the current branch. The bar is hidden entirely when no
//! PR is present — see `has_bottom_bar` and `ui::render`.
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
//! - check counts — hybrid display (see `build_line_from_info`).

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::state::{AppState, MergeableStatus, PrDisplayInfo, PrState, PrStatus};
use crate::theme::Theme;
use crate::ui::fit;

/// The bottom bar is only rendered when a PR exists against the current
/// branch. When no PR is present the row is reclaimed by the main pane.
pub fn has_bottom_bar(state: &AppState) -> bool {
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
    build_pr_line_fitted(pr_state, theme, u16::MAX as usize)
}

/// Width-aware variant of [`build_pr_line`]. Tries progressively
/// compact renditions until one fits `max_width`, falling back to the
/// most compact form if none fits.
pub fn build_pr_line_fitted(
    pr_state: &PrState,
    theme: &Theme,
    max_width: usize,
) -> Option<Line<'static>> {
    let info = pr_state.info.as_ref()?;
    for tier in 0..=3u8 {
        let line = render_tier(info, theme, tier);
        let w = line_width(&line);
        if w <= max_width {
            return Some(line);
        }
    }
    Some(render_tier(info, theme, 3))
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| fit::display_width(s.content.as_ref()))
        .sum()
}

fn render_tier(info: &PrDisplayInfo, theme: &Theme, tier: u8) -> Line<'static> {
    let state_color = pr_state_color(&info.state);
    let sep_style = Style::default().fg(theme.header_separator);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(" "));

    // PR prefix: drop "PR " at tier ≥ 3.
    let pr_label = if tier >= 3 { "#" } else { "PR #" };
    spans.push(Span::styled(
        format!("{pr_label}{}", info.number),
        Style::default()
            .fg(state_color)
            .add_modifier(Modifier::BOLD),
    ));

    // Mergeable indicator: icon always (when known); drop text at tier ≥ 1.
    if let Some((icon, label, color)) = mergeable_indicator(&info.mergeable) {
        spans.push(Span::styled("  ", sep_style));
        spans.push(Span::styled(icon.to_string(), Style::default().fg(color)));
        if tier < 1 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(label.to_string(), Style::default().fg(color)));
        }
    }

    // Checks.
    let checks = &info.checks;
    if checks.total > 0 {
        if tier >= 2 {
            // Compact: single colored fraction.
            let (icon, color) = if checks.failed > 0 {
                ("✗", Color::Red)
            } else if checks.pending > 0 {
                ("◐", Color::Yellow)
            } else {
                ("✓", Color::Green)
            };
            spans.push(Span::styled("  ", sep_style));
            spans.push(Span::styled(
                format!("{icon} {}/{}", checks.passed, checks.total),
                Style::default().fg(color),
            ));
        } else if checks.failed > 0 {
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
            let completed = checks.passed + checks.skipped;
            spans.push(Span::styled("  ", sep_style));
            spans.push(Span::styled("◐ ", Style::default().fg(Color::Yellow)));
            spans.push(Span::styled(
                format!("{}/{}", completed, checks.total),
                Style::default().fg(Color::Yellow),
            ));
        } else {
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

/// Render the bottom status bar into `area`. Expects a 1-row Rect.
///
/// Callers must only invoke this when `has_bottom_bar(state)` returns
/// `true`. The row is a single left-aligned PR status line.
pub fn render_pr_line(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    if let Some(line) =
        build_pr_line_fitted(state.pr_state(), theme, area.width as usize)
    {
        frame.render_widget(Paragraph::new(line), area);
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
    fn test_has_bottom_bar_false_without_pr() {
        let s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!has_bottom_bar(&s));
    }

    #[test]
    fn test_has_bottom_bar_true_with_pr_info() {
        let s = state_with(make_info(MergeableStatus::Clean, 12, 0, 0));
        assert!(has_bottom_bar(&s));
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

    #[test]
    fn test_pr_line_drops_mergeable_text_on_narrow_width() {
        let info = make_info(MergeableStatus::Clean, 12, 0, 0);
        let pr = pr_state_with(info);
        let line = build_pr_line_fitted(&pr, &test_theme(), 20).unwrap();
        let text = line_text(&line);
        assert!(!text.contains(" clean"), "got: {text}");
        assert!(text.contains('✓'), "icon should remain, got: {text}");
        assert!(text.contains("12/12"), "got: {text}");
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
