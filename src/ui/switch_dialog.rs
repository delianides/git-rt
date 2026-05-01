//! Modal dialog for switching which worktree git-rt is watching.
//!
//! State and key-handling live here. Rendering is in `render()` below
//! (added in Task 6). Unit tests cover the headless core; rendering is
//! validated manually.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use crate::fuzzy::Matcher;
use crate::git::worktree::WorktreeEntry;

/// One row in the dialog.
#[derive(Debug, Clone)]
pub struct Row {
    pub entry: WorktreeEntry,
    /// True for the worktree git-rt is currently watching.
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

pub struct SwitchDialog {
    rows: Vec<Row>,
    filter: String,
    selected: usize,
    filtered_indices: Vec<usize>,
    matcher: Matcher,
}

impl SwitchDialog {
    /// Build a new dialog from a list of entries and the currently watched path.
    /// `current_canonical` should already be canonicalised when possible
    /// to handle macOS `/var` → `/private/var` symlinks.
    pub fn new(entries: Vec<WorktreeEntry>, current_canonical: &Path) -> Self {
        let mut rows: Vec<Row> = entries
            .into_iter()
            .map(|entry| {
                let is_current = canonical_or_raw(&entry.path) == current_canonical;
                let label = render_label(&entry);
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

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn filter(&self) -> &str {
        &self.filter
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
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
            KeyCode::Char(c) => {
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
fn render_label(entry: &WorktreeEntry) -> String {
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
    format!("{name}  {}", entry.path.display())
}

fn canonical_or_raw(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
        let d = SwitchDialog::new(entries, Path::new("/a"));
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
        let d = SwitchDialog::new(entries, Path::new("/b"));
        assert!(d.rows()[0].is_current);
        assert_eq!(d.rows()[0].entry.branch.as_deref(), Some("feat"));
    }

    #[test]
    fn typing_filters_the_list() {
        let entries = vec![
            entry("/a", Some("main"), false),
            entry("/b", Some("feature-x"), false),
        ];
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
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
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
        let _ = d.handle_key(key('j'));
        // Filter contains 'j', selected stays at 0 (the j-only-match in filtered).
        assert_eq!(d.filter(), "j");
    }

    #[test]
    fn typing_q_appends_to_filter_does_not_cancel() {
        let entries = vec![entry("/a", Some("query"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
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
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
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
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
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
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
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
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(
            d.handle_key(special(KeyCode::Enter)),
            Some(DialogOutcome::Switch(PathBuf::from("/b")))
        );
    }

    #[test]
    fn esc_returns_cancel_with_filter_present() {
        let entries = vec![entry("/a", Some("main"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
        let _ = d.handle_key(key('x'));
        assert_eq!(
            d.handle_key(special(KeyCode::Esc)),
            Some(DialogOutcome::Cancel)
        );
    }

    #[test]
    fn backspace_pops_filter() {
        let entries = vec![entry("/a", Some("main"), false)];
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
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
        let mut d = SwitchDialog::new(entries, Path::new("/a"));
        let _ = d.handle_key(special(KeyCode::Down));
        assert_eq!(d.selected(), 1);
        // Filter to only "alpha" — selected must clamp to 0.
        let _ = d.handle_key(key('l'));
        let _ = d.handle_key(key('p'));
        let _ = d.handle_key(key('h'));
        assert_eq!(d.filtered_indices().len(), 1);
        assert_eq!(d.selected(), 0);
    }
}
