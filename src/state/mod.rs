use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    /// Number of times the file list has been refreshed
    refresh_count: usize,
    /// When the app started (for computing "last updated N seconds ago")
    start_time: Instant,
    /// When the last refresh happened
    last_refresh: Instant,
    /// Tracks when each file last changed (for flash effect)
    flash_times: HashMap<String, Instant>,
    /// How long the flash lasts
    flash_duration: Duration,
    /// Whether the terminal window is currently focused
    focused: bool,
    /// Current branch name
    branch: String,
}

impl AppState {
    pub fn new(files: Vec<FileEntry>, flash_duration: Duration, branch: String) -> Self {
        let now = Instant::now();
        Self {
            files,
            selected: 0,
            expanded: None,
            diff_cache: HashMap::new(),
            diff_scroll: 0,
            refresh_count: 0,
            start_time: now,
            last_refresh: now,
            flash_times: HashMap::new(),
            flash_duration,
            focused: true,
            branch,
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

    pub fn refresh_count(&self) -> usize {
        self.refresh_count
    }

    pub fn last_refresh_secs(&self) -> u64 {
        self.last_refresh.elapsed().as_secs()
    }

    /// Returns true if the file at the given path is currently flashing
    pub fn is_flashing(&self, path: &str) -> bool {
        self.flash_times
            .get(path)
            .is_some_and(|t| t.elapsed() < self.flash_duration)
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn set_branch(&mut self, branch: String) {
        self.branch = branch;
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

        // Build a snapshot of old file stats for flash detection
        let old_stats: HashMap<&str, (usize, usize)> = self
            .files
            .iter()
            .map(|f| (f.path.as_str(), (f.insertions, f.deletions)))
            .collect();

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

        // Detect changed files and record flash times
        let now = Instant::now();
        for file in &new_files {
            let changed = match old_stats.get(file.path.as_str()) {
                Some(&(old_ins, old_del)) => {
                    old_ins != file.insertions || old_del != file.deletions
                }
                None => true, // new file
            };
            if changed {
                self.flash_times.insert(file.path.clone(), now);
            }
        }

        // Clean up expired flash times
        self.flash_times
            .retain(|_, t| t.elapsed() < self.flash_duration);

        self.files = new_files;
        self.refresh_count += 1;
        self.last_refresh = now;

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
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

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
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
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
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        assert!(state.expanded_path().is_none());

        state.expand_selected(FileDiff::default());
        assert_eq!(state.expanded_path(), Some("a.rs"));

        state.collapse_selected();
        assert!(state.expanded_path().is_none());
    }

    #[test]
    fn test_accessors() {
        let files = vec![make_entry("a.rs", 1, 0), make_entry("b.rs", 2, 1)];
        let state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        assert_eq!(state.files().len(), 2);
        assert_eq!(state.files()[0].path, "a.rs");
        assert_eq!(state.selected_index(), 0);
        assert_eq!(state.selected_path(), Some("a.rs".to_string()));
        assert!(!state.is_expanded("a.rs"));
        assert_eq!(state.diff_scroll(), 0);
        assert_eq!(state.refresh_count(), 0);
    }

    #[test]
    fn test_expanded_diff_returns_cached() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        assert!(state.expanded_diff().is_none());

        let diff = FileDiff {
            hunks: vec![crate::git::DiffHunk {
                header: "@@ test @@".to_string(),
                lines: vec![],
            }],
        };
        state.expand_selected(diff);

        let cached = state.expanded_diff().unwrap();
        assert_eq!(cached.hunks.len(), 1);
        assert_eq!(cached.hunks[0].header, "@@ test @@");
    }

    #[test]
    fn test_scroll_diff() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        assert_eq!(state.diff_scroll(), 0);
        state.scroll_diff_down();
        assert_eq!(state.diff_scroll(), 1);
        state.scroll_diff_down();
        assert_eq!(state.diff_scroll(), 2);
        state.scroll_diff_up();
        assert_eq!(state.diff_scroll(), 1);
        state.scroll_diff_up();
        assert_eq!(state.diff_scroll(), 0);
        state.scroll_diff_up(); // should clamp at 0
        assert_eq!(state.diff_scroll(), 0);
    }

    #[test]
    fn test_navigation_empty_list() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.selected_index(), 0);
        assert!(state.selected_path().is_none());
        state.select_next(); // should not panic
        state.select_previous();
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_update_clamps_selection_when_list_shrinks() {
        let files = vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 2, 1),
            make_entry("c.rs", 0, 3),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.select_next();
        state.select_next(); // select c.rs (index 2)

        // Shrink to 1 file — selection should clamp
        state.update_files(vec![make_entry("x.rs", 1, 1)]);
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_update_collapses_when_expanded_file_removed() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.expand_selected(FileDiff::default());
        assert!(state.expanded_path().is_some());

        // Update with a list that doesn't contain a.rs
        state.update_files(vec![make_entry("b.rs", 2, 0)]);
        assert!(state.expanded_path().is_none());
    }

    #[test]
    fn test_update_increments_refresh_count() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.refresh_count(), 0);

        state.update_files(vec![make_entry("a.rs", 1, 0)]);
        assert_eq!(state.refresh_count(), 1);

        state.update_files(vec![]);
        assert_eq!(state.refresh_count(), 2);
    }

    #[test]
    fn test_flash_on_new_file() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!state.is_flashing("a.rs"));

        state.update_files(vec![make_entry("a.rs", 1, 0)]);
        assert!(state.is_flashing("a.rs"));
    }

    #[test]
    fn test_flash_on_changed_stats() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        // Initial load doesn't flash (no previous state to compare)
        assert!(!state.is_flashing("a.rs"));

        // Update with changed stats — should flash
        state.update_files(vec![make_entry("a.rs", 5, 2)]);
        assert!(state.is_flashing("a.rs"));
    }

    #[test]
    fn test_no_flash_on_unchanged_stats() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        // Update with same stats — should not flash
        state.update_files(vec![make_entry("a.rs", 1, 0)]);
        assert!(!state.is_flashing("a.rs"));
    }

    #[test]
    fn test_flash_expires() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(1), "main".to_string()); // 1ms flash

        state.update_files(vec![make_entry("a.rs", 5, 2)]);
        // Sleep just past the flash duration
        std::thread::sleep(Duration::from_millis(5));
        assert!(!state.is_flashing("a.rs"));
    }

    #[test]
    fn test_diff_cache_invalidated_on_file_removal() {
        let files = vec![make_entry("a.rs", 1, 0), make_entry("b.rs", 2, 1)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        // Expand a.rs to populate cache
        state.expand_selected(FileDiff::default());
        assert!(state.expanded_diff().is_some());

        // Remove a.rs from file list
        state.update_files(vec![make_entry("b.rs", 2, 1)]);
        // expanded should be collapsed and diff cache cleared for a.rs
        assert!(state.expanded_path().is_none());
    }

    #[test]
    fn test_focus_state() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(state.is_focused()); // focused by default

        let mut state = state;
        state.set_focused(false);
        assert!(!state.is_focused());

        state.set_focused(true);
        assert!(state.is_focused());
    }
}
