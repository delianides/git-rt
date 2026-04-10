//! UI rendering module.
//!
//! Provides the [`Terminal`] wrapper and the main `render` function that draws
//! bordered panes, the file list, inline diffs, the diff overlay, and the
//! optional PR widget.

pub mod diff_overlay;
pub mod pr_widget;

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
/// 1. If the diff overlay is visible, render the main pane first, then the
///    overlay on top.
/// 2. If PR data is available and enabled, split the layout according to
///    `config.pr.layout`.
/// 3. Otherwise render the main pane full-size.
fn render(frame: &mut Frame, state: &AppState, config: &AppConfig, theme: &Theme) {
    let area = frame.area();

    let pr_has_data = {
        let pr = state.pr_state();
        pr.info.is_some() || pr.loading || pr.error.is_some()
    };
    let pr_enabled = config.pr.enabled && pr_has_data;

    // Determine main pane area and optional PR area
    let (main_area, pr_area) = if pr_enabled && config.pr.layout != "tab" {
        split_for_pr(area, &config.pr.layout)
    } else {
        (area, None)
    };

    render_main_pane(frame, state, config, theme, main_area);

    if let Some(pr_rect) = pr_area {
        pr_widget::render_pr_widget(
            frame,
            state.pr_state(),
            config.pr.show_labels,
            theme,
            pr_rect,
        );
    }

    // Overlay goes on top of everything
    if state.is_overlay_visible() {
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

/// Split the area for a PR pane depending on layout mode.
fn split_for_pr(area: Rect, layout: &str) -> (Rect, Option<Rect>) {
    match layout {
        "right" => {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        }
        _ => {
            // "bottom" is the default
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        }
    }
}

// ── Main pane ────────────────────────────────────────────────────────────────

/// Render the main pane: bordered box with header, containing the file list
/// and optional inline diff.
fn render_main_pane(
    frame: &mut Frame,
    state: &AppState,
    config: &AppConfig,
    theme: &Theme,
    area: Rect,
) {
    let header = build_header(state, theme, area.width);

    let border_color = if state.is_focused() {
        theme.border_focused
    } else {
        theme.border
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(header)
        .title_style(Style::default().fg(theme.header_text));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    render_file_list(frame, state, config, theme, inner);
}

/// Build the header line, progressively dropping elements if the window is too narrow.
/// Priority (kept longest): repo name, file count > diff stats > branch > worktree
fn build_header(state: &AppState, theme: &Theme, width: u16) -> Line<'static> {
    let files = state.files();
    let file_count = files.len();
    let total_ins: usize = files.iter().map(|f| f.insertions).sum();
    let total_del: usize = files.iter().map(|f| f.deletions).sum();

    let repo = state.repo_name();
    let branch = state.branch().to_string();
    let worktree = state.worktree_name().to_string();
    let show_worktree = !worktree.is_empty() && worktree != repo;

    // Calculate widths of each segment (including separators)
    let repo_w = if repo.is_empty() { 1 } else { repo.len() + 4 }; // " repo · "
    let files_w = format!("{} files", file_count).len();
    let stats_w = format!(" · +{} -{}", total_ins, total_del).len();
    let branch_w = if branch.is_empty() {
        0
    } else {
        branch.len() + 3 // " · branch"
    };
    let wt_w = if show_worktree {
        worktree.len() + 3 // " · worktree"
    } else {
        0
    };
    let trailing = 2; // " " + border

    let available = width as usize;

    // Determine what fits, dropping from the end
    let full = repo_w + files_w + stats_w + branch_w + wt_w + trailing;
    let include_worktree = show_worktree && full <= available;
    let without_wt = repo_w + files_w + stats_w + branch_w + trailing;
    let include_branch = !branch.is_empty() && without_wt <= available;
    let without_branch = repo_w + files_w + stats_w + trailing;
    let include_stats = without_branch <= available;

    let sep_style = Style::default().fg(theme.header_separator);
    let text_style = Style::default().fg(theme.header_text);

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Repo name (always shown)
    if !repo.is_empty() {
        spans.push(Span::styled(format!(" {}", repo), text_style));
        spans.push(Span::styled(" · ", sep_style));
    } else {
        spans.push(Span::raw(" "));
    }

    // File count (always shown)
    spans.push(Span::styled(format!("{} files", file_count), text_style));

    // Diff stats
    if include_stats {
        spans.push(Span::styled(" · ", sep_style));
        spans.push(Span::styled(
            format!("+{}", total_ins),
            Style::default().fg(theme.file_insertions),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{}", total_del),
            Style::default().fg(theme.file_deletions),
        ));
    }

    // Branch
    if include_branch {
        spans.push(Span::styled(" · ", sep_style));
        spans.push(Span::styled(branch, text_style));
    }

    // Worktree
    if include_worktree {
        spans.push(Span::styled(" · ", sep_style));
        spans.push(Span::styled(worktree, text_style));
    }

    spans.push(Span::raw(" "));

    Line::from(spans)
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
