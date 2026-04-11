//! PR tab body wrapper. Delegates to `pr_widget::render_pr_content`.

use ratatui::{layout::Rect, Frame};

use crate::state::AppState;
use crate::theme::Theme;
use crate::ui::pr_widget::render_pr_content;

/// Render the PR tab body into `area`. This is the full inner area of the
/// main pane on the PR tab — `render_pr_content` draws without an outer
/// block because the main pane already provides one.
pub fn render_pr_tab(
    frame: &mut Frame,
    state: &AppState,
    show_labels: bool,
    theme: &Theme,
    area: Rect,
) {
    render_pr_content(frame, state.pr_state(), show_labels, theme, area);
}
