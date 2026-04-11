//! Render the Commits tab body — a list of `<short-sha>  <title>` rows.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::state::AppState;
use crate::theme::Theme;

/// Render the Commits tab body into `area`. Assumes `area` is the inner area
/// of a bordered pane (the outer `Block` is rendered by `ui::render`).
pub fn render_commits_list(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let cts = state.commits_tab();

    // 1-row top padding inside the pane, same pattern as render_file_list.
    let inner = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    if cts.commits.is_empty() {
        if inner.height < 1 || inner.width < 10 {
            return;
        }
        let msg = match cts.base_ref.as_deref() {
            None => "  No base branch found. Configure an upstream for this branch, or create a main/master branch.",
            Some(_) => "  No commits on this branch yet.",
        };
        let p = Paragraph::new(msg).style(Style::default().fg(theme.empty_text));
        frame.render_widget(p, inner);
        return;
    }

    let mut items: Vec<ListItem> = cts
        .commits
        .iter()
        .map(|c| {
            let line = Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    c.sha_short.clone(),
                    Style::default()
                        .fg(theme.header_separator)
                        .add_modifier(Modifier::DIM),
                ),
                Span::raw("  "),
                Span::styled(c.title.clone(), Style::default().fg(theme.file_path)),
            ]);
            ListItem::new(line)
        })
        .collect();

    if cts.truncated_count > 0 {
        let footer = Line::from(vec![Span::styled(
            format!("  … and {} more older commits", cts.truncated_count),
            Style::default().fg(theme.empty_text),
        )]);
        items.push(ListItem::new(footer));
    }

    let highlight = if state.is_focused() {
        Style::default()
            .bg(theme.selection_bg)
            .fg(theme.selection_fg)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(highlight);

    let mut list_state = ListState::default();
    list_state.select(Some(cts.selected_index));

    frame.render_stateful_widget(list, inner, &mut list_state);
}
