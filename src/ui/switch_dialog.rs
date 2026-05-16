//! Modal dialog for switching which worktree perch is watching.
//!
//! State, key-handling, and rendering live here. Unit tests cover the
//! headless core; the rendering function is validated manually.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::fuzzy::Matcher;
use crate::git::worktree::WorktreeEntry;

/// One row in the dialog.
#[derive(Debug, Clone)]
pub struct Row {
    pub entry: WorktreeEntry,
    /// True for the worktree perch is currently watching.
    pub is_current: bool,
    /// Pre-rendered "branch  path" (or "(detached abc1234)  path") used both
    /// for display and as the fuzzy-match haystack.
    pub label: String,
}

/// What the caller (App) should do after a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogOutcome {
    /// Cancel and close the dialog.
    Cancel,
    /// Switch to this worktree, then close the dialog.
    Switch(PathBuf),
    /// Don't act — surface a status-line message and keep the dialog open.
    Reject(String),
}

/// Modal dialog state for switching the watched worktree.
///
/// Tracks the rows to display, the current filter, the selection, the
/// filter-output index list, and the fuzzy matcher. Use [`SwitchDialog::new`]
/// to construct and [`SwitchDialog::handle_key`] to drive it.
pub struct SwitchDialog {
    rows: Vec<Row>,
    filter: String,
    selected: usize,
    filtered_indices: Vec<usize>,
    matcher: Matcher,
}

impl SwitchDialog {
    /// Build a new dialog from a list of entries, the currently watched path,
    /// and the primary worktree root.
    ///
    /// `current_canonical` should already be canonicalised when possible
    /// to handle macOS `/var` → `/private/var` symlinks.
    ///
    /// `primary_root` is used to render descendant worktree paths as relative
    /// (e.g. `./.worktrees/feat`) instead of absolute.
    pub fn new(entries: Vec<WorktreeEntry>, current_canonical: &Path, primary_root: &Path) -> Self {
        let mut rows: Vec<Row> = entries
            .into_iter()
            .map(|entry| {
                let is_current = canonical_or_raw(&entry.path) == current_canonical;
                let label = render_label(&entry, primary_root);
                Row {
                    entry,
                    is_current,
                    label,
                }
            })
            .collect();

        // Sort: current first, then non-current in given order. (Spec calls
        // for primary-second + timestamp-desc; deferring the timestamp sort
        // to a future polish.)
        rows.sort_by_key(|r| !r.is_current);

        let filtered_indices = (0..rows.len()).collect();
        Self {
            rows,
            filter: String::new(),
            selected: 0,
            filtered_indices,
            matcher: Matcher::new(),
        }
    }

    /// All rows, in dialog-display order (current first, then porcelain order).
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    /// Current fuzzy-filter input.
    pub fn filter(&self) -> &str {
        &self.filter
    }
    /// Index into [`Self::filtered_indices`] for the currently highlighted row.
    /// When the filtered list is empty, this is `0` and the renderer should
    /// check [`Self::filtered_indices`] before drawing a selection cursor.
    pub fn selected(&self) -> usize {
        self.selected
    }
    /// Indices into [`Self::rows`] that match the current filter, sorted by
    /// descending fuzzy-match score (or original order when the filter is empty).
    pub fn filtered_indices(&self) -> &[usize] {
        &self.filtered_indices
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<DialogOutcome> {
        match key.code {
            KeyCode::Esc => Some(DialogOutcome::Cancel),
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                None
            }
            KeyCode::Down => {
                if self.selected + 1 < self.filtered_indices.len() {
                    self.selected += 1;
                }
                None
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.recompute_filter();
                None
            }
            KeyCode::Enter => {
                let &idx = self.filtered_indices.get(self.selected)?;
                let row = &self.rows[idx];
                if row.is_current {
                    return Some(DialogOutcome::Cancel);
                }
                if let Some(reason) = row.entry.prunable.as_deref() {
                    return Some(DialogOutcome::Reject(format!(
                        "worktree pruned ({reason}); can't switch"
                    )));
                }
                Some(DialogOutcome::Switch(row.entry.path.clone()))
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.filter.push(c);
                self.recompute_filter();
                None
            }
            _ => None,
        }
    }

    fn recompute_filter(&mut self) {
        self.filtered_indices = self.matcher.rank(&self.filter, &self.rows, |r| &r.label);
        if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len().saturating_sub(1);
        }
    }
}

/// Render a one-line label for a worktree row.
fn render_label(entry: &WorktreeEntry, primary_root: &Path) -> String {
    let head = if entry.head.len() >= 7 {
        &entry.head[..7]
    } else {
        entry.head.as_str()
    };
    let name = match (&entry.branch, entry.detached) {
        (Some(b), _) => b.clone(),
        (None, true) => format!("(detached {head})"),
        (None, false) => "(unknown)".to_string(),
    };
    let display_path = display_path(&entry.path, primary_root);
    format!("{name}  {display_path}")
}

/// Render `path` relative to `primary_root` when it is a (canonicalised)
/// descendant of `primary_root`, otherwise return the absolute display.
fn display_path(path: &Path, primary_root: &Path) -> String {
    let path_canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root_canonical =
        std::fs::canonicalize(primary_root).unwrap_or_else(|_| primary_root.to_path_buf());
    match path_canonical.strip_prefix(&root_canonical) {
        Ok(rel) if !rel.as_os_str().is_empty() => format!("./{}", rel.display()),
        // strip_prefix succeeds with an empty Path if path == root; render as "."
        Ok(_) => ".".to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn canonical_or_raw(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Render the switch dialog as a centered modal overlay.
///
/// Caller is responsible for rendering the main pane first; this function
/// draws on top.
pub fn render(frame: &mut ratatui::Frame, dialog: &SwitchDialog, theme: &crate::theme::Theme) {
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};

    let area = frame.area();
    let overlay = centered_rect(60, 50, area);

    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focused))
        .title(" Switch worktree ")
        .title_style(Style::default().fg(theme.header_text));
    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    // Vertical layout: 1-row filter, the rest is the list.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // Filter input.
    let filter_line = Line::from(vec![
        Span::styled("> ", Style::default().fg(theme.diff_line_number)),
        Span::raw(dialog.filter().to_string()),
        Span::styled("▍", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    frame.render_widget(Paragraph::new(filter_line), chunks[0]);

    // List of filtered rows.
    let items: Vec<ListItem> = dialog
        .filtered_indices()
        .iter()
        .enumerate()
        .map(|(visible_idx, &row_idx)| {
            let row = &dialog.rows()[row_idx];
            let mut style = Style::default();
            if row.entry.prunable.is_some() {
                style = style.add_modifier(Modifier::DIM);
            }
            let gutter = if row.is_current { "* " } else { "  " };
            let lock = if row.entry.locked.is_some() {
                " 🔒"
            } else {
                ""
            };
            let line = format!("{gutter}{}{lock}", row.label);
            let mut item = ListItem::new(Line::from(Span::styled(line, style)));
            if visible_idx == dialog.selected() {
                item = item.style(Style::default().bg(theme.flash_bg));
            }
            item
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, chunks[1]);
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let w = area.width * percent_x / 100;
    let h = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    ratatui::layout::Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn entry(path: &str, branch: Option<&str>, prunable: bool) -> WorktreeEntry {
        WorktreeEntry {
            path: PathBuf::from(path),
            head: "0000000000000000000000000000000000000000".to_string(),
            branch: branch.map(|s| s.to_string()),
            bare: false,
            detached: false,
            locked: None,
            prunable: if prunable {
                Some("gitdir missing".into())
            } else {
                None
            },
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn special(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn opens_with_first_row_selected() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feat"), false),
        ];
        let d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        assert_eq!(d.selected(), 0);
        assert_eq!(d.filtered_indices(), &[0, 1]);
        assert!(d.rows()[0].is_current);
    }

    #[test]
    fn current_worktree_sorts_first() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feat"), false),
        ];
        // Current is /b — it should sort to position 0.
        let d = SwitchDialog::new(entries, Path::new("/b"), Path::new("/"));
        assert!(d.rows()[0].is_current);
        assert_eq!(d.rows()[0].entry.branch.as_deref(), Some("feat"));
    }

    #[test]
    fn typing_filters_the_list() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feature-x"), false),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        assert!(d.handle_key(key('f')).is_none());
        // Only "feature-x" matches "f".
        assert_eq!(d.filtered_indices().len(), 1);
        assert_eq!(
            d.rows()[d.filtered_indices()[0]].entry.branch.as_deref(),
            Some("feature-x")
        );
    }

    #[test]
    fn typing_j_appends_to_filter_does_not_navigate() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feat"), false),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        let _ = d.handle_key(key('j'));
        // Neither row's label contains 'j', so the filter narrows to zero matches.
        // The 'j' key cannot navigate even when the filtered list is empty.
        assert_eq!(d.filter(), "j");
        assert!(d.filtered_indices().is_empty());
        assert_eq!(d.selected(), 0);
    }

    #[test]
    fn typing_q_appends_to_filter_does_not_cancel() {
        let entries = vec![entry("/a", Some("query"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        let outcome = d.handle_key(key('q'));
        assert!(outcome.is_none(), "got {outcome:?}");
        assert_eq!(d.filter(), "q");
    }

    #[test]
    fn arrow_keys_navigate_filtered_view() {
        let entries = vec![
            entry("/a", Some("alpha"), false),
            entry("/b", Some("beta"), false),
            entry("/c", Some("gamma"), false),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        assert_eq!(d.selected(), 0);
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(d.selected(), 1);
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(d.selected(), 2);
        // Past end clamps.
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(d.selected(), 2);
        let _ = d.handle_key(special(KeyCode::Up));
        assert_eq!(d.selected(), 1);
    }

    #[test]
    fn enter_on_current_returns_cancel() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feat"), false),
        ];
        // /a is both current AND first after sort.
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        assert_eq!(
            d.handle_key(special(KeyCode::Enter)),
            Some(DialogOutcome::Cancel)
        );
    }

    #[test]
    fn enter_on_prunable_returns_reject() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("ghost"), true),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        // Move to second row.
        let _ = d.handle_key(special(KeyCode::Down));
        let outcome = d.handle_key(special(KeyCode::Enter));
        assert!(
            matches!(outcome, Some(DialogOutcome::Reject(_))),
            "got {outcome:?}"
        );
    }

    #[test]
    fn enter_on_normal_returns_switch() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feat"), false),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(
            d.handle_key(special(KeyCode::Enter)),
            Some(DialogOutcome::Switch(PathBuf::from("/b")))
        );
    }

    #[test]
    fn esc_returns_cancel_with_filter_present() {
        let entries = vec![entry("/a", Some("main"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        let _ = d.handle_key(key('x'));
        assert_eq!(
            d.handle_key(special(KeyCode::Esc)),
            Some(DialogOutcome::Cancel)
        );
    }

    #[test]
    fn backspace_pops_filter() {
        let entries = vec![entry("/a", Some("main"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        let _ = d.handle_key(key('a'));
        let _ = d.handle_key(key('b'));
        assert_eq!(d.filter(), "ab");
        let _ = d.handle_key(special(KeyCode::Backspace));
        assert_eq!(d.filter(), "a");
    }

    #[test]
    fn filtered_view_clamps_selected() {
        let entries = vec![
            entry("/a", Some("alpha"), false),
            entry("/b", Some("beta"), false),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(d.selected(), 1);
        // Filter to only "alpha" — selected must clamp to 0.
        let _ = d.handle_key(key('l'));
        let _ = d.handle_key(key('p'));
        let _ = d.handle_key(key('h'));
        assert_eq!(d.filtered_indices().len(), 1);
        assert_eq!(d.selected(), 0);
    }

    #[test]
    fn label_shows_path_relative_to_primary_when_descendant() {
        // Use the actual filesystem so canonicalize works. tempfile keeps it portable.
        let tmp = tempfile::tempdir().unwrap();
        let primary = tmp.path();
        let wt_path = primary.join(".worktrees").join("feat");
        std::fs::create_dir_all(&wt_path).unwrap();

        let entries = vec![WorktreeEntry {
            path: wt_path.clone(),
            head: "0000000000000000000000000000000000000000".to_string(),
            branch: Some("feat-x".to_string()),
            bare: false,
            detached: false,
            locked: None,
            prunable: None,
        }];
        let d = SwitchDialog::new(entries, primary, primary);
        // The label should contain "./.worktrees/feat" rather than the absolute tempdir path.
        let label = &d.rows()[0].label;
        assert!(
            label.contains("./.worktrees/feat"),
            "label should be relative under primary, got {label:?}"
        );
        assert!(
            !label.contains(primary.to_string_lossy().as_ref()),
            "label should not contain the absolute primary path, got {label:?}"
        );
    }

    #[test]
    fn label_shows_absolute_path_when_not_descendant() {
        let entries = vec![WorktreeEntry {
            path: PathBuf::from("/some/other/place/wt"),
            head: "0000000000000000000000000000000000000000".to_string(),
            branch: Some("orphan".to_string()),
            bare: false,
            detached: false,
            locked: None,
            prunable: None,
        }];
        let d = SwitchDialog::new(entries, Path::new("/primary"), Path::new("/primary"));
        let label = &d.rows()[0].label;
        assert!(
            label.contains("/some/other/place/wt"),
            "non-descendant label should remain absolute, got {label:?}"
        );
    }

    #[test]
    fn ctrl_modifier_char_does_not_corrupt_filter() {
        let entries = vec![entry("/a", Some("main"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        // Pressing Ctrl-c (or any modifier-chord character) must NOT append
        // to the filter. The App-level intercept handles Ctrl-c as quit;
        // the dialog itself defends against future reorderings.
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let outcome = d.handle_key(key);
        assert!(outcome.is_none());
        assert_eq!(d.filter(), "");
    }
}
