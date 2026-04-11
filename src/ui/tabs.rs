//! Tab bar widget and label/visibility helpers for the tabbed main pane.
//!
//! Public API:
//! - [`visible_tabs`] — compute the ordered list of (tab, label) pairs
//!   currently visible.
//! - [`render_tab_bar`] — render the tab bar into a single-row Rect.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
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

/// Render the tab bar row. Expects a 1-row-tall `area`.
pub fn render_tab_bar(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
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

    let paragraph = Paragraph::new(Line::from(spans));
    frame.render_widget(paragraph, area);
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
}
