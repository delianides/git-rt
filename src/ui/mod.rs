//! UI rendering module.
//!
//! Provides the [`Terminal`] wrapper and the main `render` function that
//! draws the bordered file list (with `repo/branch` + file/branch stats
//! in the top border title). A 1-row bottom bar is rendered below the
//! main pane only when a PR exists against the current branch; otherwise
//! the main pane takes the full frame.

pub mod colors;
pub mod diff_overlay;
pub mod fit;
pub mod header;
pub mod help_overlay;
pub mod pr_line;
pub mod switch_dialog;
pub mod tree;

use anyhow::Result;
use crossterm::{
    event::{DisableFocusChange, EnableFocusChange},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::io;

use crate::config::AppConfig;
use crate::git::{ChangeGroup, FileStatus};
use crate::state::{AppState, ViewMode};
use crate::ui::tree::{RowId, VisibleRow};

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

    /// Draw one frame.
    pub fn draw(&mut self, state: &mut AppState, config: &AppConfig) -> Result<()> {
        self.terminal.draw(|frame| {
            render(frame, state, config);
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
#[tracing::instrument(name = "ui.render", skip_all)]
fn render(frame: &mut Frame, state: &mut AppState, config: &AppConfig) {
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
        colors::FLASH_BORDER
    } else if has_pr {
        pr_border_color_from_state(state)
    } else if state.is_focused() {
        colors::BORDER_FOCUSED
    } else {
        colors::BORDER
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(header::build_header_title_with_width(
            state,
            (main_area.width as usize).saturating_sub(2),
        ));

    let inner = block.inner(main_area);
    frame.render_widget(block, main_area);

    render_file_list(frame, state, config, inner);

    // 2. Bottom bar (only when a PR exists).
    if let Some(bottom) = bottom_bar_area {
        pr_line::render_pr_line(frame, state, bottom);
    }

    // 3. Diff overlay.
    if state.is_diff_overlay_visible() {
        state.set_diff_viewport_height(diff_overlay::inner_height(frame.area()));
        if let (Some(diff), Some(path)) = (state.expanded_diff(), state.expanded_diff_path()) {
            let (ins, del) = state.expanded_diff_stats().unwrap_or((0, 0));
            diff_overlay::render_diff_overlay(frame, diff, path, ins, del, state.diff_scroll());
        }
    }

    // 4. Switch dialog (above diff, below help).
    if let Some(dialog) = state.switch_dialog() {
        switch_dialog::render(frame, dialog);
    }

    // 5. Help overlay.
    if state.is_help_visible() {
        help_overlay::render_help_overlay(frame);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return the main pane border color derived from the current PR state.
/// Falls back to the default border color when no PR data exists
/// (though the caller normally guards on `has_pr` first).
fn pr_border_color_from_state(state: &AppState) -> ratatui::style::Color {
    state
        .pr_state()
        .info
        .as_ref()
        .map(|info| pr_line::pr_state_color(&info.state))
        .unwrap_or(colors::BORDER)
}

// ── File list ────────────────────────────────────────────────────────────────

/// Render the file list.
fn render_file_list(frame: &mut Frame, state: &mut AppState, config: &AppConfig, area: Rect) {
    match state.view_mode() {
        ViewMode::Condensed => render_condensed_file_list(frame, state, config, area),
        ViewMode::Tree => render_tree_file_list(frame, state, config, area),
        ViewMode::Normal => render_normal_file_list(frame, state, config, area),
    }
}

fn render_condensed_file_list(
    frame: &mut Frame,
    state: &mut AppState,
    config: &AppConfig,
    area: Rect,
) {
    // Add a 1-row top padding inside the pane
    let area = inset_file_list_area(area);

    let files = state.files();

    if files.is_empty() {
        render_empty_or_loading_state(frame, state, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();

    for file in files {
        let line = file_line(
            file.path.clone(),
            file.status.clone(),
            file.deletions,
            file.insertions,
            area.width as usize,
        );

        let mut item = ListItem::new(line);
        if config.display.flash_on_change && state.is_flashing(&file.path) {
            item = item.style(Style::default().bg(colors::FLASH_BG));
        }
        items.push(item);
    }

    render_list(frame, state, config, area, items, state.selected_index());
}

fn render_tree_file_list(frame: &mut Frame, state: &mut AppState, config: &AppConfig, area: Rect) {
    let area = inset_file_list_area(area);
    let rows = state.visible_rows();

    if rows.is_empty() {
        render_empty_or_loading_state(frame, state, area);
        return;
    }

    let mut items = Vec::with_capacity(rows.len());

    for row in &rows {
        let line = match row {
            VisibleRow::Directory {
                depth,
                label,
                expanded,
                ..
            } => {
                let indent_cols = 2 * depth;
                let arrow = if *expanded { "▼" } else { "▶" };
                // leading space + indent + arrow (1) + " " + label + trailing margin
                let fixed = 1 + 1 + 1 + 1;
                let width = area.width as usize;
                let elastic = width.saturating_sub(fixed + indent_cols);
                let label_display = fit::middle_ellipsize(label, elastic).into_owned();

                let indent = "  ".repeat(*depth);
                Line::from(vec![
                    Span::raw(" "),
                    Span::raw(indent),
                    Span::styled(arrow, Style::default().fg(colors::FILE_PATH)),
                    Span::raw(" "),
                    Span::styled(label_display, Style::default().fg(colors::FILE_PATH)),
                ])
            }
            VisibleRow::File {
                depth, label, file, ..
            } => {
                let indent_cols = 2 * depth;
                let status_char = file_status_char(file.status.clone());
                let status_color = file_status_color(file.status.clone());
                let stats_str = format!("-{}/+{}", file.deletions, file.insertions);
                let stats_width = fit::display_width(&stats_str);
                // leading space + indent + " " + " " + status + " "
                //   + label + " " + stats + trailing margin
                let fixed_with_stats = 1 + 1 + 1 + 1 + 1 + 1 + 1;
                let fixed_no_stats = 1 + 1 + 1 + 1 + 1 + 1;
                let label_w = fit::display_width(label);
                let width = area.width as usize;
                let elastic_full =
                    width.saturating_sub(fixed_with_stats + indent_cols + stats_width);

                let (label_display, include_stats) = if label_w <= elastic_full {
                    (label.clone(), true)
                } else if elastic_full >= 20 {
                    (
                        fit::middle_ellipsize(label, elastic_full).into_owned(),
                        true,
                    )
                } else {
                    let elastic_no_stats = width.saturating_sub(fixed_no_stats + indent_cols);
                    (
                        fit::middle_ellipsize(label, elastic_no_stats).into_owned(),
                        false,
                    )
                };

                let indent = "  ".repeat(*depth);
                let mut spans = vec![
                    Span::raw(" "),
                    Span::raw(indent),
                    Span::raw(" "),
                    Span::raw(" "),
                    Span::styled(status_char, Style::default().fg(status_color)),
                    Span::raw(" "),
                    Span::styled(label_display, Style::default().fg(colors::FILE_PATH)),
                ];
                if include_stats {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        format!("-{}", file.deletions),
                        Style::default().fg(colors::DELETIONS),
                    ));
                    spans.push(Span::raw("/"));
                    spans.push(Span::styled(
                        format!("+{}", file.insertions),
                        Style::default().fg(colors::INSERTIONS),
                    ));
                }
                Line::from(spans)
            }
            VisibleRow::Header { .. } => unreachable!("tree mode emits no header rows"),
        };

        let mut item = ListItem::new(line);
        if let Some(file) = row.file() {
            if config.display.flash_on_change && state.is_flashing(&file.path) {
                item = item.style(Style::default().bg(colors::FLASH_BG));
            }
        }
        items.push(item);
    }

    render_list(frame, state, config, area, items, state.selected_index());
}

fn render_normal_file_list(
    frame: &mut Frame,
    state: &mut AppState,
    config: &AppConfig,
    area: Rect,
) {
    let area = inset_file_list_area(area);

    let rows = state.visible_rows();
    if rows.is_empty() {
        render_empty_or_loading_state(frame, state, area);
        return;
    }

    // Each group header is preceded by a non-selectable blank spacer row
    // (except the first group), so sections read as distinct blocks. The
    // spacers are render-only — they are not part of the selectable row
    // model — so `selected_index()` (a model-row index) is translated to the
    // spacer-inclusive index of the rendered item list.
    let selected = state.selected_index();
    let mut rendered_selected = 0;
    let mut items: Vec<ListItem> = Vec::with_capacity(rows.len() + 3);
    for (i, row) in rows.iter().enumerate() {
        if row.is_header() && !items.is_empty() {
            items.push(ListItem::new(Line::from("")));
        }
        if i == selected {
            rendered_selected = items.len();
        }
        let item = match row {
            VisibleRow::Header {
                id,
                label,
                count,
                collapsed,
            } => {
                let group = match id {
                    RowId::Group(g) => *g,
                    _ => unreachable!("normal header row must carry a group id"),
                };
                let color = section_header_color(group);
                let arrow = if *collapsed { "▶" } else { "▼" };
                ListItem::new(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(arrow, Style::default().fg(color)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{label} ({count})"),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ]))
            }
            VisibleRow::File { file, .. } => {
                let line = file_line(
                    file.path.clone(),
                    file.status.clone(),
                    file.deletions,
                    file.insertions,
                    area.width as usize,
                );
                let mut item = ListItem::new(line);
                if config.display.flash_on_change && state.is_flashing(&file.path) {
                    item = item.style(Style::default().bg(colors::FLASH_BG));
                }
                item
            }
            VisibleRow::Directory { .. } => {
                unreachable!("normal mode emits no directory rows")
            }
        };
        items.push(item);
    }

    render_list(frame, state, config, area, items, rendered_selected);
}

/// The color for a status-group header in the Normal view.
fn section_header_color(group: ChangeGroup) -> ratatui::style::Color {
    match group {
        ChangeGroup::Changes => colors::SECTION_CHANGES,
        ChangeGroup::New => colors::SECTION_NEW,
        ChangeGroup::Committed => colors::SECTION_COMMITTED,
    }
}

fn inset_file_list_area(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    }
}

fn render_empty_or_loading_state(frame: &mut Frame, state: &mut AppState, area: Rect) {
    state.set_scroll_offset(0);
    if area.height < 2 || area.width < 20 {
        return;
    }
    if state.is_computing() {
        let loading = Paragraph::new("Loading\u{2026}")
            .style(Style::default().add_modifier(ratatui::style::Modifier::DIM))
            .alignment(Alignment::Center);
        frame.render_widget(loading, area);
        return;
    }
    let msg = Paragraph::new("  No changes detected. Watching for file changes...")
        .style(Style::default().fg(colors::EMPTY_TEXT));
    frame.render_widget(msg, area);
}

fn render_list(
    frame: &mut Frame,
    state: &mut AppState,
    config: &AppConfig,
    area: Rect,
    items: Vec<ListItem<'static>>,
    selected_index: usize,
) {
    let max_selected_index = selected_index.min(items.len().saturating_sub(1));

    let highlight = if state.is_focused() {
        // A gray bar with a bright fg so the row stays legible even where the
        // text is itself gray; per-span colors go monochrome on this row only.
        colors::SELECTION
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(highlight)
        .scroll_padding(config.display.scroll_padding);

    let mut list_state = ListState::default()
        .with_offset(state.scroll_offset())
        .with_selected(Some(max_selected_index));

    frame.render_stateful_widget(list, area, &mut list_state);

    state.set_scroll_offset(list_state.offset());
    state.set_list_viewport_height(area.height as usize);
}

fn file_line(
    label: String,
    status: FileStatus,
    deletions: usize,
    insertions: usize,
    available_width: usize,
) -> Line<'static> {
    let status_char = file_status_char(status.clone());
    let status_color = file_status_color(status);

    let stats_str = format!("-{deletions}/+{insertions}");
    let stats_width = fit::display_width(&stats_str);
    // Fixed cost with stats: leading space + status + space + path
    // + space + stats + trailing margin. Excludes path and stats.
    let fixed_with_stats = 1 + 1 + 1 + 1 + 1;
    // Fixed cost without stats: leading space + status + space + path
    // + trailing margin.
    let fixed_no_stats = 1 + 1 + 1 + 1;

    let path_width = fit::display_width(&label);
    let elastic_full = available_width.saturating_sub(fixed_with_stats + stats_width);

    let (path_display, include_stats) = if path_width <= elastic_full {
        (label.clone(), true)
    } else if elastic_full >= 20 {
        (
            fit::middle_ellipsize(&label, elastic_full).into_owned(),
            true,
        )
    } else {
        let elastic_no_stats = available_width.saturating_sub(fixed_no_stats);
        (
            fit::middle_ellipsize(&label, elastic_no_stats).into_owned(),
            false,
        )
    };

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(status_char, Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(path_display, Style::default().fg(colors::FILE_PATH)),
    ];

    if include_stats {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{deletions}"),
            Style::default().fg(colors::DELETIONS),
        ));
        spans.push(Span::styled("/", Style::default().fg(colors::FILE_PATH)));
        spans.push(Span::styled(
            format!("+{insertions}"),
            Style::default().fg(colors::INSERTIONS),
        ));
    }

    Line::from(spans)
}

fn file_status_char(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Modified => "M",
        FileStatus::Added => "A",
        FileStatus::Deleted => "D",
        FileStatus::Renamed => "R",
        FileStatus::Untracked => "?",
        FileStatus::Staged => "S",
        FileStatus::Conflicted => "C",
    }
}

fn file_status_color(status: FileStatus) -> ratatui::style::Color {
    match status {
        FileStatus::Modified => colors::STATUS_MODIFIED,
        FileStatus::Added => colors::STATUS_ADDED,
        FileStatus::Deleted => colors::STATUS_DELETED,
        FileStatus::Renamed => colors::STATUS_RENAMED,
        FileStatus::Untracked => colors::STATUS_UNTRACKED,
        FileStatus::Staged => colors::STATUS_STAGED,
        FileStatus::Conflicted => colors::STATUS_CONFLICTED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{ChangeGroup, FileEntry, FileStatus};
    use ratatui::backend::TestBackend;
    use std::time::Duration;

    fn make_entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 1,
            group: ChangeGroup::Changes,
        }
    }

    fn render_to_string(
        state: &mut AppState,
        config: &AppConfig,
        width: u16,
        height: u16,
    ) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state, config)).unwrap();

        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    /// Render to a `TestBackend` and return each row as its own string.
    fn render_rows(
        state: &mut AppState,
        config: &AppConfig,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state, config)).unwrap();

        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect()
    }

    #[test]
    fn test_no_row_exceeds_pane_width_across_matrix() {
        use ratatui::backend::TestBackend;

        let files = vec![
            FileEntry {
                path: "src/very/deeply/nested/path/to/a_really_long_filename.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 234,
                deletions: 15,
                group: ChangeGroup::Changes,
            },
            FileEntry {
                path: "Cargo.toml".to_string(),
                status: FileStatus::Modified,
                insertions: 2,
                deletions: 1,
                group: ChangeGroup::Changes,
            },
        ];

        for &width in &[80u16, 60, 40, 30, 24, 20] {
            for &tree in &[false, true] {
                let mut state = AppState::new(
                    files.clone(),
                    Duration::from_millis(600),
                    "feat/very-long-branch-name-for-testing".to_string(),
                );
                state.set_repo_name("perch".to_string());
                if tree {
                    state.cycle_view_mode();
                }

                let backend = TestBackend::new(width, 12);
                let mut terminal = ratatui::Terminal::new(backend).unwrap();
                terminal
                    .draw(|frame| render(frame, &mut state, &AppConfig::default()))
                    .unwrap_or_else(|e| {
                        panic!("render must not panic at width {width} tree={tree}: {e}")
                    });
            }
        }
    }

    #[test]
    fn test_tree_directory_row_ellipsizes_label_at_narrow_width() {
        let files = vec![
            make_entry("some/very/long/directory/path/a.rs"),
            make_entry("some/very/long/directory/path/b.rs"),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        let rendered = render_to_string(&mut state, &AppConfig::default(), 28, 8);
        assert!(
            rendered.contains('\u{2026}'),
            "expected ellipsis, got: {rendered}"
        );
        assert!(
            !rendered.contains("some/very/long/directory/path/"),
            "got: {rendered}"
        );
    }

    #[test]
    fn test_tree_file_row_ellipsizes_label_at_narrow_width() {
        let files = vec![
            make_entry("src/ui/really_long_filename_here.rs"),
            make_entry("src/ui/mod.rs"),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        let rendered = render_to_string(&mut state, &AppConfig::default(), 30, 8);
        assert!(
            rendered.contains('\u{2026}'),
            "expected ellipsis, got: {rendered}"
        );
    }

    #[test]
    fn test_condensed_row_mid_ellipsizes_path_at_narrow_width() {
        let files = vec![FileEntry {
            path: "src/very/deeply/nested/path/long_filename.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 234,
            deletions: 15,
            group: ChangeGroup::Changes,
        }];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        let rendered = render_to_string(&mut state, &AppConfig::default(), 50, 6);
        assert!(rendered.contains("long_filename.rs"), "got: {rendered}");
        assert!(rendered.contains("-15"), "got: {rendered}");
        assert!(rendered.contains("+234"), "got: {rendered}");
        assert!(rendered.contains('\u{2026}'), "got: {rendered}");
    }

    #[test]
    fn test_condensed_row_drops_stats_below_floor() {
        let files = vec![FileEntry {
            path: "src/a/b/c/d/really_long_filename.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 234,
            deletions: 15,
            group: ChangeGroup::Changes,
        }];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        // Width 24: with borders + padding the elastic budget drops
        // under the 20-col floor, so stats should be dropped from the
        // file row (the header title may still show aggregate stats).
        let rendered_rows = render_rows(&mut state, &AppConfig::default(), 24, 6);
        let file_row = rendered_rows
            .iter()
            .find(|r| r.contains(" M "))
            .expect("file row present");
        assert!(
            !file_row.contains("-15"),
            "stats should drop, got: {file_row}"
        );
        assert!(
            !file_row.contains("+234"),
            "stats should drop, got: {file_row}"
        );
        assert!(
            file_row.contains(".rs"),
            "filename extension should remain, got: {file_row}"
        );
        assert!(
            file_row.contains('\u{2026}'),
            "ellipsis expected, got: {file_row}"
        );
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

        let backend = TestBackend::new(40, 22);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &mut state, &config))
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

    #[test]
    fn test_render_tree_mode_shows_directory_arrows_and_filename_only_rows() {
        let files = vec![make_entry("src/ui/mod.rs"), make_entry("src/ui/header.rs")];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        let rendered = render_to_string(&mut state, &AppConfig::default(), 50, 12);
        assert!(rendered.contains("▼ src/ui/"));
        assert!(rendered.contains("mod.rs"));
        assert!(!rendered.contains("src/ui/mod.rs"));
    }

    #[test]
    fn test_render_tree_mode_shows_loading_state_when_no_rows() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();
        state.set_computing(true);

        let rendered = render_to_string(&mut state, &AppConfig::default(), 50, 12);
        assert!(rendered.contains("Loading…"));
    }

    #[test]
    fn test_render_tree_mode_shows_empty_state_when_no_rows() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        let rendered = render_to_string(&mut state, &AppConfig::default(), 60, 12);
        assert!(rendered.contains("No changes detected. Watching for file changes..."));
    }

    #[test]
    fn test_render_diff_overlay_uses_stored_diff_file_metadata() {
        let files = vec![
            FileEntry {
                path: "src/ui/mod.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 7,
                deletions: 3,
                group: ChangeGroup::Changes,
            },
            FileEntry {
                path: "src/ui/header.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 2,
                deletions: 1,
                group: ChangeGroup::Changes,
            },
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_expanded_diff("src/ui/mod.rs".to_string(), crate::git::FileDiff::default());
        state.show_diff_overlay();
        state.select_next();

        let rendered = render_to_string(&mut state, &AppConfig::default(), 60, 16);
        assert!(rendered.contains("src/ui/mod.rs +7 -3"));
        assert!(!rendered.contains("src/ui/header.rs +2 -1"));
    }

    fn grouped_entry(path: &str, group: ChangeGroup) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 1,
            group,
        }
    }

    #[test]
    fn test_render_normal_mode_shows_group_headers_with_arrow_and_count() {
        let files = vec![
            grouped_entry("src/ui/changed.rs", ChangeGroup::Changes),
            grouped_entry("src/ui/committed.rs", ChangeGroup::Committed),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        // Resolve a base so the no-base guard is skipped.
        state.set_merge_base(
            Some(gix::ObjectId::empty_tree(gix::hash::Kind::Sha1)),
            "main".to_string(),
        );

        let rendered = render_to_string(&mut state, &AppConfig::default(), 60, 14);
        assert!(
            rendered.contains("▼"),
            "expected expanded arrow, got: {rendered}"
        );
        assert!(
            rendered.contains("Changes (1)"),
            "expected Changes header with count, got: {rendered}"
        );
        assert!(
            rendered.contains("Committed (1)"),
            "expected Committed header with count, got: {rendered}"
        );
    }

    #[test]
    fn test_render_normal_mode_pads_between_groups() {
        let files = vec![
            grouped_entry("src/ui/changed.rs", ChangeGroup::Changes),
            grouped_entry("src/ui/committed.rs", ChangeGroup::Committed),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        state.set_merge_base(
            Some(gix::ObjectId::empty_tree(gix::hash::Kind::Sha1)),
            "main".to_string(),
        );

        let rows = render_rows(&mut state, &AppConfig::default(), 60, 14);
        let committed_idx = rows
            .iter()
            .position(|r| r.contains("Committed (1)"))
            .expect("Committed header should render");
        // The second group gets one blank line of padding above its header.
        let padding = &rows[committed_idx - 1];
        assert!(
            padding.trim_matches(|c| c == ' ' || c == '│').is_empty(),
            "expected a blank padding line above the Committed header, got: {padding:?}"
        );
        let changes_idx = rows
            .iter()
            .position(|r| r.contains("Changes (1)"))
            .expect("Changes header should render");
        assert!(
            changes_idx < committed_idx,
            "Changes group should render before Committed"
        );
    }

    #[test]
    fn test_render_normal_mode_collapsed_group_hides_its_files() {
        let files = vec![grouped_entry("src/ui/changed.rs", ChangeGroup::Changes)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        state.set_merge_base(
            Some(gix::ObjectId::empty_tree(gix::hash::Kind::Sha1)),
            "main".to_string(),
        );
        // Row 0 is the "Changes" header; collapse it.
        assert!(state.toggle_selected_group());

        let rendered = render_to_string(&mut state, &AppConfig::default(), 60, 14);
        assert!(
            rendered.contains("Changes (1)"),
            "header should remain, got: {rendered}"
        );
        assert!(
            rendered.contains("▶"),
            "expected collapsed arrow, got: {rendered}"
        );
        assert!(
            !rendered.contains("changed.rs"),
            "collapsed group should hide its files, got: {rendered}"
        );
    }

    #[test]
    fn test_render_normal_mode_renders_groups_without_base() {
        // Without a resolved base, Normal must still render the base-independent
        // groups (Changes, New). The Committed group is silently absent.
        let files = vec![
            grouped_entry("src/ui/changed.rs", ChangeGroup::Changes),
            grouped_entry("README.md", ChangeGroup::New),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        // Seed the first snapshot so `initial_seed_done()` is true; leave
        // `merge_base()` as `None`.
        state.update_files(vec![
            grouped_entry("src/ui/changed.rs", ChangeGroup::Changes),
            grouped_entry("README.md", ChangeGroup::New),
        ]);
        assert!(state.initial_seed_done());
        assert!(state.merge_base().is_none());

        let rendered = render_to_string(&mut state, &AppConfig::default(), 60, 14);
        assert!(
            rendered.contains("Changes (1)"),
            "expected Changes header without a base, got: {rendered}"
        );
        assert!(
            rendered.contains("New files (1)"),
            "expected New files header without a base, got: {rendered}"
        );
        assert!(
            !rendered.contains("Committed"),
            "Committed group must be hidden when no base is resolved, got: {rendered}"
        );
        assert!(
            !rendered.contains("needs a base branch"),
            "no-base bail-out must be removed, got: {rendered}"
        );
    }
}
