//! Tab bar label/visibility helpers for the tabbed main pane.
//!
//! Public API:
//! - [`visible_tabs`] — compute the ordered list of (tab, label) pairs
//!   currently visible.
//! - [`tab_bar_title`] — build the styled `Line` shown in the main
//!   pane's top border (via `Block::title`).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::state::{AppState, Tab};
use crate::theme::Theme;

/// Return the ordered list of visible tabs with their display labels.
///
/// The order is always `[Changes, Commits]`, optionally followed by `Pr`
/// when PR data has been loaded.
pub fn visible_tabs(state: &AppState) -> Vec<(Tab, String)> {
    let mut out: Vec<(Tab, String)> = vec![
        (Tab::Changes, "Changes".to_string()),
        (Tab::Commits, "Commits".to_string()),
    ];
    if state.is_pr_tab_visible() {
        let label = match state.pr_state().info.as_ref() {
            Some(info) => format!("PR #{}", info.number),
            None => "PR".to_string(),
        };
        out.push((Tab::Pr, label));
    }
    out
}

/// Build the styled `Line` for the main pane's tab bar.
///
/// This `Line` is passed to `Block::title()` so the tabs render **inside
/// the top border** of the main pane, reclaiming the row that was
/// previously a dedicated tab-bar strip above the pane.
///
/// Layout rules:
/// - The line starts with a single leading space so the first label doesn't
///   butt up against the rounded `╭` corner.
/// - The active tab is preceded by a `●` bullet and rendered with reversed
///   fg/bg (`selection_bg` / `selection_fg` + `BOLD`). Inactive tabs get a
///   two-space indent so labels stay column-aligned regardless of which
///   tab is active — this avoids a horizontal jump on `Tab` keypresses.
/// - Tabs are separated by a single space.
/// - A trailing space follows the last label so the title doesn't butt up
///   against the right border either.
pub fn tab_bar_title(state: &AppState, theme: &Theme) -> Line<'static> {
    let tabs = visible_tabs(state);
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];

    for (i, (tab, label)) in tabs.iter().enumerate() {
        let is_active = *tab == state.active_tab();
        if is_active {
            spans.push(Span::styled(
                "● ".to_string(),
                Style::default().fg(theme.header_text),
            ));
            spans.push(Span::styled(
                label.clone(),
                Style::default()
                    .bg(theme.selection_bg)
                    .fg(theme.selection_fg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("  {label}"),
                Style::default().fg(theme.header_text),
            ));
        }
        if i < tabs.len() - 1 {
            spans.push(Span::raw(" "));
        }
    }

    spans.push(Span::raw(" "));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChecksInfo, MergeableStatus, PrDisplayInfo, PrStatus};
    use std::time::Duration;

    fn pr_info(number: u64) -> PrDisplayInfo {
        PrDisplayInfo {
            number,
            title: "t".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 0,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        }
    }

    #[test]
    fn test_visible_tabs_without_pr() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        let tabs = visible_tabs(&state);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].0, Tab::Changes);
        assert_eq!(tabs[0].1, "Changes");
        assert_eq!(tabs[1].0, Tab::Commits);
        assert_eq!(tabs[1].1, "Commits");
    }

    #[test]
    fn test_visible_tabs_with_pr_number() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_pr_info(pr_info(142));
        let tabs = visible_tabs(&state);
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[2].0, Tab::Pr);
        assert_eq!(tabs[2].1, "PR #142");
    }

    /// Collect the plain-text content of a `Line` by concatenating all of
    /// its spans' contents. Handy for asserting the rendered layout without
    /// inspecting individual styles.
    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    /// Build a real `Theme` for tests — `tab_bar_title` doesn't care about
    /// specific colors, it just needs a non-panicking Theme instance.
    fn test_theme() -> Theme {
        crate::theme::load_theme(crate::theme::DEFAULT_THEME_NAME, None)
    }

    #[test]
    fn test_tab_bar_title_without_pr() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        let line = tab_bar_title(&state, &test_theme());
        // Active tab (Changes) gets the bullet; inactive (Commits) gets the
        // two-space indent. Wrapped in single leading/trailing spaces so the
        // border corners don't crowd the labels.
        assert_eq!(line_text(&line), " ● Changes   Commits ");
    }

    #[test]
    fn test_tab_bar_title_with_pr() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_pr_info(pr_info(142));
        let line = tab_bar_title(&state, &test_theme());
        assert_eq!(line_text(&line), " ● Changes   Commits   PR #142 ");
    }

    #[test]
    fn test_tab_bar_title_active_follows_active_tab() {
        // Switch to Commits — the bullet must move with it, and Changes
        // should fall back to the inactive two-space indent.
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_tab(Tab::Commits);
        let line = tab_bar_title(&state, &test_theme());
        assert_eq!(line_text(&line), "   Changes ● Commits ");
    }
}
