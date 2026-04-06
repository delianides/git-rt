use std::collections::HashMap;

use crate::git::{FileDiff, FileEntry};

/// The application's view model — what the UI renders from
pub struct AppState {
    /// All changed files in the repo
    files: Vec<FileEntry>,
    /// Currently selected index in the file list
    selected: usize,
    /// The path of the currently expanded file (if any)
    expanded: Option<String>,
    /// Cached diffs keyed by file path
    diff_cache: HashMap<String, FileDiff>,
    /// Scroll offset within the expanded diff
    diff_scroll: usize,
}

impl AppState {
    pub fn new(files: Vec<FileEntry>) -> Self {
        Self {
            files,
            selected: 0,
            expanded: None,
            diff_cache: HashMap::new(),
            diff_scroll: 0,
        }
    }

    // -- Accessors --

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_path(&self) -> Option<String> {
        self.files.get(self.selected).map(|f| f.path.clone())
    }

    pub fn is_expanded(&self, path: &str) -> bool {
        self.expanded.as_deref() == Some(path)
    }

    pub fn expanded_path(&self) -> Option<&str> {
        self.expanded.as_deref()
    }

    pub fn expanded_diff(&self) -> Option<&FileDiff> {
        self.expanded
            .as_ref()
            .and_then(|path| self.diff_cache.get(path))
    }

    pub fn diff_scroll(&self) -> usize {
        self.diff_scroll
    }

    // -- Navigation --

    pub fn select_next(&mut self) {
        if !self.files.is_empty() {
            self.selected = (self.selected + 1).min(self.files.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    // -- Expand / Collapse --

    /// Expand the selected file's diff, collapsing any previously expanded file
    pub fn expand_selected(&mut self, diff: FileDiff) {
        if let Some(path) = self.selected_path() {
            self.diff_cache.insert(path.clone(), diff);
            self.expanded = Some(path);
            self.diff_scroll = 0;
        }
    }

    /// Collapse the currently expanded file
    pub fn collapse_selected(&mut self) {
        self.expanded = None;
        self.diff_scroll = 0;
    }

    // -- Diff scrolling --

    pub fn scroll_diff_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(1);
    }

    pub fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(1);
    }

    // -- State updates --

    /// Update the file list from a fresh git status computation.
    /// Preserves selection position and expanded state where possible.
    pub fn update_files(&mut self, new_files: Vec<FileEntry>) {
        // Try to preserve the selected file by path
        let selected_path = self.selected_path();

        // Invalidate diff cache for files that are no longer present
        // or whose stats have changed
        let new_paths: std::collections::HashSet<&str> =
            new_files.iter().map(|f| f.path.as_str()).collect();

        self.diff_cache
            .retain(|path, _| new_paths.contains(path.as_str()));

        // If the expanded file is gone, collapse
        if let Some(ref expanded) = self.expanded {
            if !new_paths.contains(expanded.as_str()) {
                self.expanded = None;
                self.diff_scroll = 0;
            }
        }

        self.files = new_files;

        // Restore selection by path, or clamp to valid range
        if let Some(prev_path) = selected_path {
            self.selected = self
                .files
                .iter()
                .position(|f| f.path == prev_path)
                .unwrap_or(self.selected.min(self.files.len().saturating_sub(1)));
        } else {
            self.selected = self.selected.min(self.files.len().saturating_sub(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileStatus;

    fn make_entry(path: &str, ins: usize, del: usize) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: ins,
            deletions: del,
        }
    }

    #[test]
    fn test_navigation() {
        let files = vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 2, 1),
            make_entry("c.rs", 0, 3),
        ];
        let mut state = AppState::new(files);

        assert_eq!(state.selected_index(), 0);
        state.select_next();
        assert_eq!(state.selected_index(), 1);
        state.select_next();
        assert_eq!(state.selected_index(), 2);
        state.select_next(); // should clamp
        assert_eq!(state.selected_index(), 2);
        state.select_previous();
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn test_update_preserves_selection() {
        let files = vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 2, 1),
            make_entry("c.rs", 0, 3),
        ];
        let mut state = AppState::new(files);
        state.select_next(); // select b.rs

        let new_files = vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 5, 2), // changed stats
            make_entry("d.rs", 1, 1), // c.rs gone, d.rs new
        ];
        state.update_files(new_files);

        // Should still have b.rs selected
        assert_eq!(state.selected_path().unwrap(), "b.rs");
    }

    #[test]
    fn test_expand_collapse() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files);

        assert!(state.expanded_path().is_none());

        state.expand_selected(FileDiff::default());
        assert_eq!(state.expanded_path(), Some("a.rs"));

        state.collapse_selected();
        assert!(state.expanded_path().is_none());
    }
}
