//! UI rendering module.
//!
//! Provides the [`Terminal`] wrapper and the main `render` function that draws
//! the tab bar, a bordered main pane dispatched by active tab, the status line,
//! and tab-aware diff overlays.

pub mod diff_overlay;
pub mod pr_tab;
pub mod pr_widget;
pub mod status_line;
pub mod tabs;

use anyhow::Result;
use crossterm::{
    event::{DisableFocusChange, EnableFocusChange},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::io;

use crate::config::AppConfig;
use crate::git::{DiffLineKind, FileStatus};
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
    pub fn draw(&mut self, state: &AppState, config: &AppConfig, theme: &Theme) -> Result<()> {
        self.terminal.draw(|frame| {
            render(frame, state, config, theme);
        })?;
        Ok(())
    }
}

// ── Main render entry point ──────────────────────────────────────────────────

/// Top-level render function.
///
/// Splits the frame into two regions:
/// 1. Main pane (bordered block whose top-border title is the tab bar;
///    body is dispatched by active tab)
/// 2. Status line (1 row)
///
/// Then draws a tab-aware diff overlay on top when appropriate.
fn render(frame: &mut Frame, state: &AppState, config: &AppConfig, theme: &Theme) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // main pane
            Constraint::Length(1), // status line
        ])
        .split(area);

    let main_area = chunks[0];
    let status_area = chunks[1];

    // 1. Main pane border color
    let border_color = if state.is_border_flashing() {
        theme.flash_bg
    } else if state.active_tab() == crate::state::Tab::Pr {
        pr_border_color_for_state(state, theme)
    } else if state.is_focused() {
        theme.border_focused
    } else {
        theme.border
    };

    // Tabs render inside the block's top border via `.title(...)`.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(tabs::tab_bar_title(state, theme));

    let inner = block.inner(main_area);
    frame.render_widget(block, main_area);

    // 3. Active tab body dispatch
    match state.active_tab() {
        crate::state::Tab::Changes => {
            render_file_list(frame, state, config, theme, inner);
        }
        crate::state::Tab::Pr => {
            pr_tab::render_pr_tab(frame, state, config.pr.show_labels, theme, inner);
        }
    }

    // 4. Status line
    status_line::render_status_line(frame, state, theme, status_area);

    // 5. Overlay (drawn on top of everything). Only the Changes tab has
    // an overlay; the PR tab is read-only.
    if state.active_tab() == crate::state::Tab::Changes && state.is_overlay_visible() {
        if let Some(diff) = state.expanded_diff() {
            let path = state.expanded_path().unwrap_or("");
            let (ins, del) = state
                .files()
                .iter()
                .find(|f| f.path == path)
                .map(|f| (f.insertions, f.deletions))
                .unwrap_or((0, 0));
            diff_overlay::render_diff_overlay(
                frame,
                diff,
                path,
                ins,
                del,
                state.diff_scroll(),
                theme,
            );
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return the border color for the PR tab based on the current PR state.
fn pr_border_color_for_state(state: &AppState, theme: &Theme) -> ratatui::style::Color {
    use crate::state::PrStatus;
    use ratatui::style::Color;
    match state.pr_state().info.as_ref().map(|i| &i.state) {
        Some(PrStatus::Open) => Color::Green,
        Some(PrStatus::Closed) => Color::Red,
        Some(PrStatus::Merged) => Color::Magenta,
        Some(PrStatus::Draft) => Color::Gray,
        None => theme.border,
    }
}

// ── File list ────────────────────────────────────────────────────────────────

/// Render the file list with optional inline diffs.
fn render_file_list(
    frame: &mut Frame,
    state: &AppState,
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
        if area.height < 2 || area.width < 20 {
            return;
        }
        let msg = Paragraph::new("  No changes detected. Watching for file changes...")
            .style(Style::default().fg(theme.empty_text));
        frame.render_widget(msg, area);
        return;
    }

    let use_inline = config.keys.enter == "inline";

    let mut items: Vec<ListItem> = Vec::new();
    let mut list_index_to_file_index: Vec<Option<usize>> = Vec::new();

    for (i, file) in files.iter().enumerate() {
        let is_expanded = state.is_expanded(&file.path);

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
            FileStatus::Modified => theme.file_path,
            FileStatus::Added => theme.file_insertions,
            FileStatus::Deleted => theme.file_deletions,
            FileStatus::Untracked => theme.empty_text,
            FileStatus::Staged => theme.file_insertions,
            FileStatus::Renamed => theme.header_text,
            FileStatus::Conflicted => theme.flash_bg,
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
            Span::raw(" "),
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

        // Inline diff (only when enter mode is "inline")
        if use_inline && is_expanded {
            if let Some(diff) = state.expanded_diff() {
                let inline_lines = build_inline_diff_lines(diff, theme);
                for il in inline_lines {
                    items.push(ListItem::new(il));
                    list_index_to_file_index.push(None);
                }
            }
        }
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
        .highlight_style(highlight);

    let mut list_state = ListState::default();
    list_state.select(Some(selected_list_index));

    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Build the lines for an inline diff display.
///
/// Uses thick side borders (┃) and thin top/bottom (─) to create a nested
/// bordered appearance.
fn build_inline_diff_lines<'a>(diff: &crate::git::FileDiff, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    let border_style = Style::default().fg(theme.diff_border);

    // Top border
    lines.push(Line::from(Span::styled(
        "  ┌────────────────────────────────────────────────────────",
        border_style,
    )));

    for hunk in &diff.hunks {
        // Hunk header
        lines.push(Line::from(vec![
            Span::styled("  ┃ ", border_style),
            Span::styled(
                hunk.header.clone(),
                Style::default()
                    .fg(theme.diff_hunk_header)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        for diff_line in &hunk.lines {
            let (prefix, style) = match diff_line.kind {
                DiffLineKind::Addition => (
                    "+",
                    Style::default().fg(theme.diff_add_fg).bg(theme.diff_add_bg),
                ),
                DiffLineKind::Deletion => (
                    "-",
                    Style::default().fg(theme.diff_del_fg).bg(theme.diff_del_bg),
                ),
                DiffLineKind::Context => (" ", Style::default().fg(theme.diff_context)),
                DiffLineKind::HunkHeader => ("@", Style::default().fg(theme.diff_hunk_header)),
            };

            lines.push(Line::from(vec![
                Span::styled("  ┃ ", border_style),
                Span::styled(format!("{prefix} {}", diff_line.content), style),
            ]));
        }
    }

    // Bottom border
    lines.push(Line::from(Span::styled(
        "  └────────────────────────────────────────────────────────",
        border_style,
    )));

    lines
}
