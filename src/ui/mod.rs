pub mod format;

use anyhow::Result;
use crossterm::{
    event::{DisableFocusChange, EnableFocusChange},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::io;

use crate::config::DisplayConfig;
use crate::git::DiffLineKind;
use crate::state::AppState;

pub struct Terminal {
    terminal: ratatui::Terminal<CrosstermBackend<io::Stdout>>,
}

impl Terminal {
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = ratatui::Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn setup(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableFocusChange)?;
        self.terminal.clear()?;
        Ok(())
    }

    pub fn teardown(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), DisableFocusChange, LeaveAlternateScreen)?;
        Ok(())
    }

    pub fn draw(&mut self, state: &AppState, display: &DisplayConfig) -> Result<()> {
        self.terminal.draw(|frame| {
            render(frame, state, display);
        })?;
        Ok(())
    }
}

/// Main render function
fn render(frame: &mut Frame, state: &AppState, display: &DisplayConfig) {
    let area = frame.area();

    // Layout: file list takes up available space, status bar at bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    render_file_list(frame, state, display, chunks[0]);
    render_status_bar(frame, state, display, chunks[1]);
}

/// Render the file list with optional expanded diff
fn render_file_list(frame: &mut Frame, state: &AppState, display: &DisplayConfig, area: Rect) {
    let pad = &display.padding;
    // Apply vertical padding to the area, but keep full width for highlight bar
    let area = Rect {
        x: area.x,
        y: area.y + pad.top,
        width: area.width,
        height: area.height.saturating_sub(pad.top + pad.bottom),
    };
    let left_pad = " ".repeat(pad.left as usize);

    let files = state.files();

    if files.is_empty() {
        let msg = Paragraph::new("  No changes detected. Watching for file changes...")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, area);
        return;
    }

    let segments = format::parse_format(&display.file_line);
    let widths = format::compute_column_widths(&segments, files, state.branch());

    let mut items: Vec<ListItem> = Vec::new();
    let mut list_index_to_file_index: Vec<Option<usize>> = Vec::new();

    for (i, file) in files.iter().enumerate() {
        let is_expanded = state.is_expanded(&file.path);

        // Build the file line from the format string
        // Subtract horizontal padding + marker from content width
        let marker_width: u16 = if display.show_expand_marker { 2 } else { 0 };
        let line_width = area
            .width
            .saturating_sub(marker_width + pad.left + pad.right);
        let mut line = format::render_file_line(
            &segments,
            file,
            state.branch(),
            &widths,
            line_width,
            &crate::config::StatusColors::default(),
        );

        // Prepend expand marker (after left padding)
        if display.show_expand_marker {
            let marker = if is_expanded { "▼ " } else { "  " };
            line.spans.insert(0, Span::raw(marker.to_string()));
        }

        // Prepend left padding
        if pad.left > 0 {
            line.spans.insert(0, Span::raw(left_pad.clone()));
        }

        let mut item = ListItem::new(line);
        if display.flash_on_change && state.is_flashing(&file.path) {
            item = item.style(Style::default().bg(Color::Rgb(100, 100, 30)));
        }
        items.push(item);
        list_index_to_file_index.push(Some(i));

        // If this file is expanded, insert diff lines
        if is_expanded {
            if let Some(diff) = state.expanded_diff() {
                for hunk in &diff.hunks {
                    let header_line = Line::from(vec![
                        Span::raw(format!("{}│  ", left_pad)),
                        Span::styled(hunk.header.clone(), Style::default().fg(Color::Cyan)),
                    ]);
                    items.push(ListItem::new(header_line));
                    list_index_to_file_index.push(None);

                    for diff_line in &hunk.lines {
                        let (prefix, color) = match diff_line.kind {
                            DiffLineKind::Addition => ("+", Color::Green),
                            DiffLineKind::Deletion => ("-", Color::Red),
                            DiffLineKind::Context => (" ", Color::DarkGray),
                            DiffLineKind::HunkHeader => ("@", Color::Cyan),
                        };

                        let line = Line::from(vec![
                            Span::raw(format!("{}│  ", left_pad)),
                            Span::styled(
                                format!("{prefix} {}", &diff_line.content),
                                Style::default().fg(color),
                            ),
                        ]);
                        items.push(ListItem::new(line));
                        list_index_to_file_index.push(None);
                    }
                }
            }
        }
    }

    let selected_list_index = list_index_to_file_index
        .iter()
        .position(|idx| *idx == Some(state.selected_index()))
        .unwrap_or(0);

    let highlight = if state.is_focused() {
        Style::default().bg(Color::DarkGray)
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

/// Render the bottom status bar
fn render_status_bar(frame: &mut Frame, state: &AppState, display: &DisplayConfig, area: Rect) {
    let file_count = state.files().len();
    let total_ins: usize = state.files().iter().map(|f| f.insertions).sum();
    let total_del: usize = state.files().iter().map(|f| f.deletions).sum();

    let mut spans = vec![
        Span::styled(
            format!(" {file_count} files changed"),
            Style::default().fg(Color::White),
        ),
        Span::raw("  "),
        Span::styled(format!("-{total_del}"), Style::default().fg(Color::Red)),
        Span::raw("  "),
        Span::styled(format!("+{total_ins}"), Style::default().fg(Color::Green)),
    ];

    if display.show_refresh_counter {
        let last_secs = state.last_refresh_secs();
        let refresh_count = state.refresh_count();
        spans.push(Span::raw("  │  "));
        spans.push(Span::styled(
            format!("updated {last_secs}s ago (#{refresh_count})"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    spans.push(Span::raw("  │  "));
    spans.push(Span::styled(
        "j/k:nav  enter:expand  q:quit",
        Style::default().fg(Color::DarkGray),
    ));

    let status = Line::from(spans);

    let bar = Paragraph::new(status).style(Style::default().bg(Color::Rgb(30, 30, 30)));
    frame.render_widget(bar, area);
}
