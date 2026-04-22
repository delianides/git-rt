//! UI rendering module.
//!
//! Provides the [`Terminal`] wrapper and the main `render` function that
//! draws the bordered file list (with `repo/branch` + file/branch stats
//! in the top border title). A 1-row bottom bar is rendered below the
//! main pane only when a PR exists against the current branch; otherwise
//! the main pane takes the full frame.

pub mod diff_overlay;
pub mod header;
pub mod help_overlay;
pub mod pr_line;

use anyhow::Result;
use crossterm::{
    event::{DisableFocusChange, EnableFocusChange},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::io;

use crate::config::AppConfig;
use crate::git::FileStatus;
use crate::state::AppState;
use crate::theme::Theme;

/// Wrapper around the ratatui terminal.
pub struct Terminal {
    terminal: ratatui::Terminal<CrosstermBackend<io::Stdout>>,
}

impl Terminal {
    /// Create a new terminal instance.
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = ratatui::Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    /// Enter raw mode and the alternate screen.
    pub fn setup(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableFocusChange)?;
        self.terminal.clear()?;
        Ok(())
    }

    /// Leave raw mode and restore the original screen.
    pub fn teardown(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), DisableFocusChange, LeaveAlternateScreen)?;
        Ok(())
    }

    /// Draw one frame using the new config/theme API.
    pub fn draw(&mut self, state: &mut AppState, config: &AppConfig, theme: &Theme) -> Result<()> {
        self.terminal.draw(|frame| {
            render(frame, state, config, theme);
        })?;
        Ok(())
    }

    /// Clear the screen. Used when restoring after a foreground child exits.
    pub fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }
}

// ── Main render entry point ──────────────────────────────────────────────────

/// Top-level render function.
fn render(frame: &mut Frame, state: &mut AppState, config: &AppConfig, theme: &Theme) {
    let area = frame.area();

    let has_pr = pr_line::has_bottom_bar(state);

    // Layout: when a PR exists, reserve a 1-row bottom bar; otherwise
    // the main pane takes the full frame.
    let (main_area, bottom_bar_area) = if has_pr {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    // 1. Main pane. Border color reflects PR state when a PR is open,
    // otherwise the usual flash / focused / default progression.
    let border_color = if state.is_border_flashing() {
        theme.flash_bg
    } else if has_pr {
        pr_border_color_from_state(state, theme)
    } else if state.is_focused() {
        theme.border_focused
    } else {
        theme.border
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(header::build_header_title(state, theme));

    let inner = block.inner(main_area);
    frame.render_widget(block, main_area);

    render_file_list(frame, state, config, theme, inner);

    // 2. Bottom bar (only when a PR exists).
    if let Some(bottom) = bottom_bar_area {
        pr_line::render_pr_line(frame, state, theme, bottom);
    }

    // 3. Diff overlay.
    if state.is_diff_overlay_visible() {
        if let (Some(diff), Some(path)) = (state.expanded_diff(), state.selected_path()) {
            let (ins, del) = state
                .files()
                .get(state.selected_index())
                .map(|f| (f.insertions, f.deletions))
                .unwrap_or((0, 0));
            diff_overlay::render_diff_overlay(
                frame,
                diff,
                &path,
                ins,
                del,
                state.diff_scroll(),
                theme,
            );
        }
    }

    // 4. Help overlay.
    if state.is_help_visible() {
        help_overlay::render_help_overlay(frame, theme);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return the main pane border color derived from the current PR state.
/// Falls back to the theme's default border color when no PR data exists
/// (though the caller normally guards on `has_pr` first).
fn pr_border_color_from_state(state: &AppState, theme: &Theme) -> ratatui::style::Color {
    state
        .pr_state()
        .info
        .as_ref()
        .map(|info| pr_line::pr_state_color(&info.state))
        .unwrap_or(theme.border)
}

// ── File list ────────────────────────────────────────────────────────────────

/// Render the file list.
fn render_file_list(
    frame: &mut Frame,
    state: &mut AppState,
    config: &AppConfig,
    theme: &Theme,
    area: Rect,
) {
    // Add a 1-row top padding inside the pane
    let area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let files = state.files();

    if files.is_empty() {
        state.set_scroll_offset(0);
        if area.height < 2 || area.width < 20 {
            return;
        }
        if state.is_computing() {
            use ratatui::layout::Alignment;
            let loading = Paragraph::new("Loading\u{2026}")
                .style(Style::default().add_modifier(ratatui::style::Modifier::DIM))
                .alignment(Alignment::Center);
            frame.render_widget(loading, area);
            return;
        }
        let msg = Paragraph::new("  No changes detected. Watching for file changes...")
            .style(Style::default().fg(theme.empty_text));
        frame.render_widget(msg, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut list_index_to_file_index: Vec<Option<usize>> = Vec::new();

    for (i, file) in files.iter().enumerate() {
        // Status character
        let status_char = match file.status {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Untracked => "?",
            FileStatus::Staged => "S",
            FileStatus::Conflicted => "C",
        };

        let status_color = match file.status {
            FileStatus::Modified => theme.status_modified,
            FileStatus::Added => theme.status_added,
            FileStatus::Deleted => theme.status_deleted,
            FileStatus::Renamed => theme.status_renamed,
            FileStatus::Untracked => theme.status_untracked,
            FileStatus::Staged => theme.status_staged,
            FileStatus::Conflicted => theme.status_conflicted,
        };

        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(status_char, Style::default().fg(status_color)),
            Span::raw(" "),
            Span::styled(file.path.clone(), Style::default().fg(theme.file_path)),
            Span::raw(" "),
            Span::styled(
                format!("-{}", file.deletions),
                Style::default().fg(theme.file_deletions),
            ),
            Span::raw("/"),
            Span::styled(
                format!("+{}", file.insertions),
                Style::default().fg(theme.file_insertions),
            ),
        ]);

        let mut item = ListItem::new(line);
        if config.display.flash_on_change && state.is_flashing(&file.path) {
            item = item.style(Style::default().bg(theme.flash_bg));
        }
        items.push(item);
        list_index_to_file_index.push(Some(i));
    }

    let selected_list_index = list_index_to_file_index
        .iter()
        .position(|idx| *idx == Some(state.selected_index()))
        .unwrap_or(0);

    let highlight = if state.is_focused() {
        // Only change bg so per-span fg colors (diff counts, status chars)
        // remain visible on the selected row.
        Style::default().bg(theme.selection_bg)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(highlight)
        .scroll_padding(config.display.scroll_padding);

    let mut list_state = ListState::default()
        .with_offset(state.scroll_offset())
        .with_selected(Some(selected_list_index));

    frame.render_stateful_widget(list, area, &mut list_state);

    state.set_scroll_offset(list_state.offset());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{FileEntry, FileStatus};
    use crate::theme::load_theme;
    use ratatui::backend::TestBackend;
    use std::time::Duration;

    fn make_entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 1,
        }
    }

    #[test]
    fn test_render_persists_scroll_offset() {
        // 50 files, selection at index 45, viewport ~22 rows (20 content + borders).
        let files: Vec<FileEntry> = (0..50)
            .map(|i| make_entry(&format!("file_{i:02}.rs")))
            .collect();
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        for _ in 0..45 {
            state.select_next();
        }
        assert_eq!(state.selected_index(), 45);

        let mut config = AppConfig::default();
        config.display.scroll_padding = 3;
        let theme = load_theme(crate::theme::DEFAULT_THEME_NAME, None);

        let backend = TestBackend::new(40, 22);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &mut state, &config, &theme))
            .unwrap();

        // Terminal: 22 rows. Block border: 2 rows. Top-pad inside
        // render_file_list: 1 row. Content area: 22 - 2 - 1 = 19 rows.
        // With selection=45 and scroll_padding=3, the last visible row must
        // be at index >= 48, so offset >= 48 - (19 - 1) = 30.
        let offset = state.scroll_offset();
        assert!(offset > 0, "offset should scroll forward, got {offset}");
        assert!(
            offset >= 30,
            "offset must keep selection+padding visible, got {offset}"
        );
    }
}
