//! Status line widget — one-row strip below the main pane.
//!
//! Left segment: global repo/branch/worktree context, stable across tabs.
//! Right segment: per-tab context that varies with the active tab.
//! Flash messages temporarily overwrite the right segment.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::state::AppState;
use crate::theme::Theme;

/// Build the text for the left (global) segment of the status line.
pub fn build_left_segment(state: &AppState) -> String {
    let mut parts: Vec<String> = Vec::new();
    let repo = state.repo_name();
    if !repo.is_empty() {
        parts.push(repo.to_string());
    }
    let branch = state.branch();
    if !branch.is_empty() {
        parts.push(branch.to_string());
    }
    let worktree = state.worktree_name();
    if !worktree.is_empty() && worktree != repo {
        parts.push(worktree.to_string());
    }
    if let Some((ahead, behind)) = state.ahead_behind() {
        if ahead > 0 || behind > 0 {
            parts.push(format!("↑{ahead} ↓{behind}"));
        }
    }
    let stash = state.stash_count();
    if stash > 0 {
        parts.push(format!("{stash} stash"));
    }
    parts.join(" · ")
}

/// Build the text for the right segment of the status line.
pub fn build_right_segment(state: &AppState) -> String {
    let files = state.files();
    let ins: usize = files.iter().map(|f| f.insertions).sum();
    let del: usize = files.iter().map(|f| f.deletions).sum();
    format!("{} files +{ins} -{del}", files.len())
}

/// Render the status line row. Expects a 1-row `area`.
pub fn render_status_line(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let left = build_left_segment(state);
    // Flash message overrides the right segment while active
    let right = match state.flash_message() {
        Some(msg) => msg.to_string(),
        None => build_right_segment(state),
    };

    let width = area.width as usize;
    let gap_w = width
        .saturating_sub(left.chars().count())
        .saturating_sub(right.chars().count())
        .saturating_sub(2); // leading + trailing space
    let gap: String = " ".repeat(gap_w.max(1));

    let text_style = Style::default().fg(theme.header_text);
    let sep_style = Style::default().fg(theme.header_separator);

    // Left segment with muted separators
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let mut first = true;
    for part in left.split(" · ") {
        if !first {
            spans.push(Span::styled(" · ".to_string(), sep_style));
        }
        spans.push(Span::styled(part.to_string(), text_style));
        first = false;
    }

    spans.push(Span::raw(gap));
    spans.push(Span::styled(right, text_style));
    spans.push(Span::raw(" "));

    let paragraph = Paragraph::new(Line::from(spans));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_state() -> AppState {
        let mut s = AppState::new(
            vec![],
            std::time::Duration::from_millis(600),
            "main".to_string(),
        );
        s.set_repo_name("git-rt".to_string());
        s.set_worktree_name("git-rt".to_string());
        s
    }

    #[test]
    fn test_left_segment_basic() {
        let s = fresh_state();
        let text = build_left_segment(&s);
        assert_eq!(text, "git-rt · main");
    }

    #[test]
    fn test_left_segment_with_worktree_and_ab() {
        let mut s = fresh_state();
        s.set_worktree_name("drew-branch".to_string());
        s.set_ahead_behind(Some((2, 1)));
        s.set_stash_count(3);
        let text = build_left_segment(&s);
        assert_eq!(text, "git-rt · main · drew-branch · ↑2 ↓1 · 3 stash");
    }

    #[test]
    fn test_left_segment_hides_zero_ab() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((0, 0)));
        let text = build_left_segment(&s);
        assert_eq!(text, "git-rt · main");
    }

    #[test]
    fn test_right_segment_shows_file_counts() {
        let s = fresh_state();
        let text = build_right_segment(&s);
        assert_eq!(text, "0 files +0 -0");
    }
}
