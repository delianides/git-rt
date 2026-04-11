use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::git::{FileDiff, FileEntry};

/// How the diff is displayed when Enter is pressed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewMode {
    Overlay,
    Inline,
}

/// Which tab is currently active in the main pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Changes,
    Commits,
    Pr,
}

/// State owned by the Commits tab.
#[derive(Debug, Clone, Default)]
pub struct CommitsTabState {
    /// Base ref used for the `base..HEAD` range, once resolved.
    pub base_ref: Option<String>,
    /// Commits in the range, newest-first. Capped at 100.
    pub commits: Vec<crate::git::commits::CommitEntry>,
    /// How many commits exist beyond the cap. 0 when fully loaded.
    pub truncated_count: usize,
    /// Current selection within the commits list.
    pub selected_index: usize,
    /// Whether the diff overlay is currently open for this tab.
    pub overlay_visible: bool,
    /// Scroll offset of the open overlay.
    pub diff_scroll: usize,
    /// Diff for the currently expanded commit, if any.
    pub expanded_diff: Option<crate::git::FileDiff>,
    /// Full hex sha of the expanded commit, if any.
    pub expanded_sha: Option<String>,
}

/// State of the PR widget
#[derive(Debug, Clone, Default)]
pub struct PrState {
    pub info: Option<PrDisplayInfo>,
    pub error: Option<String>,
    pub loading: bool,
}

/// Displayable PR info
#[derive(Debug, Clone)]
pub struct PrDisplayInfo {
    pub number: u64,
    pub title: String,
    pub state: PrStatus,
    pub reviews: Vec<ReviewInfo>,
    pub checks: ChecksInfo,
    pub comment_count: u64,
    pub mergeable: MergeableStatus,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrStatus {
    Open,
    Closed,
    Merged,
    Draft,
}

#[derive(Debug, Clone)]
pub struct ReviewInfo {
    pub reviewer: String,
    pub state: ReviewState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Pending,
    Commented,
    Dismissed,
}

#[derive(Debug, Clone)]
pub struct ChecksInfo {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
    pub skipped: usize,
    pub checks: Vec<CheckInfo>,
}

#[derive(Debug, Clone)]
pub struct CheckInfo {
    pub name: String,
    pub status: CheckStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Passed,
    Failed,
    Pending,
    Running,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeableStatus {
    Clean,
    Conflicts,
    Behind,
    Unknown,
}

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
    /// Repository name (basename of repo root)
    repo_name: String,
    /// Worktree name (basename of worktree path)
    worktree_name: String,
    /// HEAD short SHA
    head_sha: String,
    /// HEAD commit message (first line)
    head_message: String,
    /// Number of stash entries
    stash_count: usize,
    /// Ahead/behind upstream (ahead, behind), None if no upstream
    ahead_behind: Option<(usize, usize)>,
    /// Repo state (REBASING, MERGING, etc.), None if clean
    repo_state: Option<String>,
    /// Temporary message displayed on the bottom statusline
    flash_message: Option<(String, Instant)>,
    /// Whether the diff overlay is currently shown
    overlay_visible: bool,
    /// PR widget state
    pr_state: PrState,
    /// When set, the pane border should flash until this instant.
    border_flash_until: Option<Instant>,
    /// Which tab is currently active.
    active_tab: Tab,
    /// Owned state for the Commits tab.
    commits_tab: CommitsTabState,
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
            repo_name: String::new(),
            worktree_name: String::new(),
            head_sha: String::new(),
            head_message: String::new(),
            stash_count: 0,
            ahead_behind: None,
            repo_state: None,
            flash_message: None,
            overlay_visible: false,
            pr_state: PrState::default(),
            border_flash_until: None,
            active_tab: Tab::Changes,
            commits_tab: CommitsTabState::default(),
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

    /// Returns true if the pane border is currently in a flash state.
    pub fn is_border_flashing(&self) -> bool {
        match self.border_flash_until {
            Some(until) => Instant::now() < until,
            None => false,
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn active_tab(&self) -> Tab {
        self.active_tab
    }

    pub fn commits_tab(&self) -> &CommitsTabState {
        &self.commits_tab
    }

    pub fn commits_tab_mut(&mut self) -> &mut CommitsTabState {
        &mut self.commits_tab
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn set_branch(&mut self, branch: String) {
        self.branch = branch;
    }

    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    pub fn set_repo_name(&mut self, name: String) {
        self.repo_name = name;
    }

    pub fn worktree_name(&self) -> &str {
        &self.worktree_name
    }

    pub fn set_worktree_name(&mut self, name: String) {
        self.worktree_name = name;
    }

    pub fn head_sha(&self) -> &str {
        &self.head_sha
    }

    pub fn head_message(&self) -> &str {
        &self.head_message
    }

    pub fn set_head_info(&mut self, sha: String, message: String) {
        self.head_sha = sha;
        self.head_message = message;
    }

    pub fn stash_count(&self) -> usize {
        self.stash_count
    }

    pub fn set_stash_count(&mut self, count: usize) {
        self.stash_count = count;
    }

    pub fn ahead_behind(&self) -> Option<(usize, usize)> {
        self.ahead_behind
    }

    pub fn set_ahead_behind(&mut self, ab: Option<(usize, usize)>) {
        self.ahead_behind = ab;
    }

    pub fn repo_state(&self) -> Option<&str> {
        self.repo_state.as_deref()
    }

    pub fn set_repo_state(&mut self, state: Option<String>) {
        self.repo_state = state;
    }

    /// Get the current flash message if it hasn't expired
    pub fn flash_message(&self) -> Option<&str> {
        self.flash_message.as_ref().and_then(|(msg, time)| {
            if time.elapsed() < self.flash_duration {
                Some(msg.as_str())
            } else {
                None
            }
        })
    }

    /// Set a temporary flash message on the bottom statusline
    pub fn set_flash_message(&mut self, message: String) {
        self.flash_message = Some((message, Instant::now()));
    }

    /// Clear the flash message
    pub fn clear_flash_message(&mut self) {
        self.flash_message = None;
    }

    // -- Overlay --

    /// Returns true if the diff overlay is currently visible
    pub fn is_overlay_visible(&self) -> bool {
        self.overlay_visible
    }

    /// Show the diff overlay
    pub fn show_overlay(&mut self) {
        self.overlay_visible = true;
    }

    /// Hide the diff overlay and reset diff scroll
    pub fn hide_overlay(&mut self) {
        self.overlay_visible = false;
        self.diff_scroll = 0;
    }

    // -- PR state --

    /// Get a reference to the current PR state
    pub fn pr_state(&self) -> &PrState {
        &self.pr_state
    }

    /// Set PR info and clear loading/error state
    pub fn set_pr_info(&mut self, info: PrDisplayInfo) {
        self.pr_state.info = Some(info);
        self.pr_state.error = None;
        self.pr_state.loading = false;
    }

    /// Set a PR error and clear loading state
    pub fn set_pr_error(&mut self, error: String) {
        self.pr_state.error = Some(error);
        self.pr_state.loading = false;
    }

    /// Mark PR state as loading
    pub fn set_pr_loading(&mut self) {
        self.pr_state.loading = true;
    }

    /// Reset PR state to default
    pub fn clear_pr(&mut self) {
        self.pr_state = PrState::default();
    }

    pub fn is_pr_tab_visible(&self) -> bool {
        self.pr_state.info.is_some()
    }

    /// Return the ordered list of currently visible tabs.
    /// `Changes` and `Commits` are always present; `Pr` is only present when
    /// PR data has been loaded.
    fn visible_tabs(&self) -> Vec<Tab> {
        let mut v = vec![Tab::Changes, Tab::Commits];
        if self.is_pr_tab_visible() {
            v.push(Tab::Pr);
        }
        v
    }

    /// Activate a specific tab. Silently no-ops if the target is `Pr` and the
    /// PR tab is not visible.
    pub fn set_tab(&mut self, tab: Tab) {
        if tab == Tab::Pr && !self.is_pr_tab_visible() {
            return;
        }
        if self.active_tab != tab {
            self.reset_active_tab_transient();
            self.active_tab = tab;
        }
    }

    /// Cycle to the next visible tab. Wraps at the end.
    pub fn next_tab(&mut self) {
        let visible = self.visible_tabs();
        let current_idx = visible.iter().position(|t| *t == self.active_tab);
        let Some(idx) = current_idx else {
            // Active tab no longer visible — fall back to Changes
            self.set_tab(Tab::Changes);
            return;
        };
        let next_idx = (idx + 1) % visible.len();
        let target = visible[next_idx];
        self.set_tab(target);
    }

    /// Cycle to the previous visible tab. Wraps at the start.
    pub fn prev_tab(&mut self) {
        let visible = self.visible_tabs();
        let current_idx = visible.iter().position(|t| *t == self.active_tab);
        let Some(idx) = current_idx else {
            self.set_tab(Tab::Changes);
            return;
        };
        let prev_idx = if idx == 0 { visible.len() - 1 } else { idx - 1 };
        let target = visible[prev_idx];
        self.set_tab(target);
    }

    /// Clear transient state on the currently-active tab. Invoked by `set_tab`
    /// just before changing `active_tab`, so callers don't need to invoke it
    /// directly. The PR tab has no transient state.
    fn reset_active_tab_transient(&mut self) {
        match self.active_tab {
            Tab::Changes => {
                self.overlay_visible = false;
                self.expanded = None;
                self.diff_scroll = 0;
                self.selected = 0;
            }
            Tab::Commits => {
                self.commits_tab.overlay_visible = false;
                self.commits_tab.expanded_diff = None;
                self.commits_tab.expanded_sha = None;
                self.commits_tab.diff_scroll = 0;
                self.commits_tab.selected_index = 0;
            }
            Tab::Pr => {}
        }
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

    /// Reset state for a worktree switch. Clears selection, expansion,
    /// diff cache, and flash times. Updates files, branch, repo name,
    /// and worktree name.
    pub fn reset_for_switch(
        &mut self,
        files: Vec<FileEntry>,
        branch: String,
        repo_name: String,
        worktree_name: String,
    ) {
        self.files = files;
        self.selected = 0;
        self.expanded = None;
        self.diff_cache.clear();
        self.diff_scroll = 0;
        self.refresh_count = 0;
        self.last_refresh = Instant::now();
        self.flash_times.clear();
        self.branch = branch;
        self.repo_name = repo_name;
        self.worktree_name = worktree_name;
        self.head_sha.clear();
        self.head_message.clear();
        self.stash_count = 0;
        self.ahead_behind = None;
        self.repo_state = None;
        self.overlay_visible = false;
        self.pr_state = PrState::default();
        // Activate border flash for visual feedback on switch
        self.border_flash_until = Some(Instant::now() + self.flash_duration);
    }

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

    #[test]
    fn test_repo_metadata_accessors() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        assert_eq!(state.repo_name(), "");
        assert_eq!(state.worktree_name(), "");
        assert_eq!(state.head_sha(), "");
        assert_eq!(state.head_message(), "");
        assert_eq!(state.stash_count(), 0);
        assert_eq!(state.ahead_behind(), None);
        assert_eq!(state.repo_state(), None);
    }

    #[test]
    fn test_set_repo_metadata() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        state.set_repo_name("git-rt".to_string());
        state.set_worktree_name("git-rt".to_string());
        state.set_head_info("abc1234".to_string(), "fix: some bug".to_string());
        state.set_stash_count(3);
        state.set_ahead_behind(Some((2, 1)));
        state.set_repo_state(Some("REBASING".to_string()));

        assert_eq!(state.repo_name(), "git-rt");
        assert_eq!(state.worktree_name(), "git-rt");
        assert_eq!(state.head_sha(), "abc1234");
        assert_eq!(state.head_message(), "fix: some bug");
        assert_eq!(state.stash_count(), 3);
        assert_eq!(state.ahead_behind(), Some((2, 1)));
        assert_eq!(state.repo_state(), Some("REBASING"));
    }

    #[test]
    fn test_flash_message_default_none() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(state.flash_message().is_none());
    }

    #[test]
    fn test_set_and_get_flash_message() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_flash_message("Switched to worktree: foo".to_string());
        assert_eq!(state.flash_message().unwrap(), "Switched to worktree: foo");
    }

    #[test]
    fn test_flash_message_expires() {
        let mut state = AppState::new(vec![], Duration::from_millis(1), "main".to_string());
        state.set_flash_message("test".to_string());
        std::thread::sleep(Duration::from_millis(5));
        assert!(state.flash_message().is_none());
    }

    #[test]
    fn test_clear_flash_message() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_flash_message("test".to_string());
        state.clear_flash_message();
        assert!(state.flash_message().is_none());
    }

    #[test]
    fn test_overlay_visibility() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!state.is_overlay_visible());
        state.show_overlay();
        assert!(state.is_overlay_visible());
        state.hide_overlay();
        assert!(!state.is_overlay_visible());
    }

    #[test]
    fn test_pr_state_default() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(state.pr_state().info.is_none());
        assert!(state.pr_state().error.is_none());
        assert!(!state.pr_state().loading);
    }

    #[test]
    fn test_set_pr_info() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_pr_loading();
        assert!(state.pr_state().loading);
        state.set_pr_info(PrDisplayInfo {
            number: 42,
            title: "feat: test".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 5,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        });
        assert!(!state.pr_state().loading);
        assert_eq!(state.pr_state().info.as_ref().unwrap().number, 42);
    }

    #[test]
    fn test_reset_clears_overlay_and_pr() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.show_overlay();
        state.set_pr_info(PrDisplayInfo {
            number: 1,
            title: "t".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 0,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        });
        state.reset_for_switch(vec![], "main".to_string(), "r".to_string(), "w".to_string());
        assert!(!state.is_overlay_visible());
        assert!(state.pr_state().info.is_none());
    }

    #[test]
    fn test_border_flash_initially_off() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!state.is_border_flashing());
    }

    #[test]
    fn test_border_flash_set_on_switch() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        state.reset_for_switch(
            vec![make_entry("b.rs", 2, 1)],
            "feature".to_string(),
            "repo".to_string(),
            "wt".to_string(),
        );
        assert!(
            state.is_border_flashing(),
            "flash should be set after reset_for_switch"
        );
    }

    #[test]
    fn test_border_flash_expires() {
        let mut state = AppState::new(vec![], Duration::from_millis(1), "main".to_string());
        state.reset_for_switch(vec![], "x".to_string(), "r".to_string(), "w".to_string());
        assert!(state.is_border_flashing());
        std::thread::sleep(Duration::from_millis(5));
        assert!(!state.is_border_flashing());
    }

    #[test]
    fn test_reset_for_switch() {
        let files = vec![make_entry("a.rs", 1, 0), make_entry("b.rs", 2, 1)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.select_next(); // select b.rs
        state.expand_selected(FileDiff::default());
        state.set_repo_name("old-repo".to_string());
        state.set_worktree_name("old-wt".to_string());

        let new_files = vec![make_entry("c.rs", 3, 0)];
        state.reset_for_switch(
            new_files,
            "feature".to_string(),
            "new-repo".to_string(),
            "new-wt".to_string(),
        );

        assert_eq!(state.selected_index(), 0);
        assert!(state.expanded_path().is_none());
        assert_eq!(state.files().len(), 1);
        assert_eq!(state.files()[0].path, "c.rs");
        assert_eq!(state.branch(), "feature");
        assert_eq!(state.repo_name(), "new-repo");
        assert_eq!(state.worktree_name(), "new-wt");
        assert_eq!(state.refresh_count(), 0);
    }

    #[test]
    fn test_tab_default_is_changes() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.active_tab(), Tab::Changes);
    }

    #[test]
    fn test_commits_tab_state_default() {
        let cts = CommitsTabState::default();
        assert!(cts.base_ref.is_none());
        assert!(cts.commits.is_empty());
        assert_eq!(cts.truncated_count, 0);
        assert_eq!(cts.selected_index, 0);
        assert!(!cts.overlay_visible);
        assert_eq!(cts.diff_scroll, 0);
        assert!(cts.expanded_diff.is_none());
        assert!(cts.expanded_sha.is_none());
    }

    #[test]
    fn test_is_pr_tab_visible_reflects_info() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!state.is_pr_tab_visible());

        state.set_pr_info(PrDisplayInfo {
            number: 1,
            title: "t".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 0,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        });
        assert!(state.is_pr_tab_visible());
    }

    #[test]
    fn test_set_tab_direct() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_tab(Tab::Commits);
        assert_eq!(state.active_tab(), Tab::Commits);
    }

    #[test]
    fn test_set_tab_pr_silently_noops_when_hidden() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_tab(Tab::Pr);
        assert_eq!(state.active_tab(), Tab::Changes);
    }

    #[test]
    fn test_next_tab_cycles_with_pr_visible() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_pr_info(pr_info_fixture());
        assert_eq!(state.active_tab(), Tab::Changes);
        state.next_tab();
        assert_eq!(state.active_tab(), Tab::Commits);
        state.next_tab();
        assert_eq!(state.active_tab(), Tab::Pr);
        state.next_tab();
        assert_eq!(state.active_tab(), Tab::Changes);
    }

    #[test]
    fn test_next_tab_skips_pr_when_hidden() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.active_tab(), Tab::Changes);
        state.next_tab();
        assert_eq!(state.active_tab(), Tab::Commits);
        state.next_tab();
        assert_eq!(state.active_tab(), Tab::Changes);
    }

    #[test]
    fn test_prev_tab_cycles_with_pr_visible() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_pr_info(pr_info_fixture());
        state.prev_tab();
        assert_eq!(state.active_tab(), Tab::Pr);
        state.prev_tab();
        assert_eq!(state.active_tab(), Tab::Commits);
        state.prev_tab();
        assert_eq!(state.active_tab(), Tab::Changes);
    }

    #[test]
    fn test_prev_tab_skips_pr_when_hidden() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.prev_tab();
        assert_eq!(state.active_tab(), Tab::Commits);
        state.prev_tab();
        assert_eq!(state.active_tab(), Tab::Changes);
    }

    fn pr_info_fixture() -> PrDisplayInfo {
        PrDisplayInfo {
            number: 42,
            title: "feat: test".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 0,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        }
    }

    #[test]
    fn test_tab_switch_resets_source_tab_transient_state() {
        let files = vec![make_entry("a.rs", 1, 0), make_entry("b.rs", 2, 1)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        // Put Changes tab in a non-default state
        state.select_next(); // select b.rs (index 1)
        state.expand_selected(FileDiff::default());
        state.show_overlay();
        state.scroll_diff_down();
        assert_eq!(state.selected_index(), 1);
        assert!(state.is_overlay_visible());
        assert_eq!(state.diff_scroll(), 1);

        // Switch to Commits — source (Changes) transient state should reset at the moment of switching
        state.set_tab(Tab::Commits);
        assert_eq!(state.active_tab(), Tab::Commits);

        // Switch back to Changes — should be at initial state
        state.set_tab(Tab::Changes);
        assert_eq!(state.selected_index(), 0);
        assert!(!state.is_overlay_visible());
        assert_eq!(state.diff_scroll(), 0);
        assert!(state.expanded_path().is_none());
    }

    #[test]
    fn test_tab_switch_resets_commits_tab_transient_state() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());

        // Manually populate Commits tab with non-default transient state
        state.commits_tab_mut().selected_index = 3;
        state.commits_tab_mut().overlay_visible = true;
        state.commits_tab_mut().diff_scroll = 5;
        state.commits_tab_mut().expanded_sha = Some("abc1234".to_string());

        // Activate Commits (set_tab resets Changes, not Commits — Commits state untouched)
        state.set_tab(Tab::Commits);
        assert_eq!(state.commits_tab().selected_index, 3);
        assert!(state.commits_tab().overlay_visible);
        assert_eq!(state.commits_tab().diff_scroll, 5);

        // Switch away from Commits — Commits transient state should now be reset
        state.set_tab(Tab::Changes);
        assert_eq!(state.commits_tab().selected_index, 0);
        assert!(!state.commits_tab().overlay_visible);
        assert_eq!(state.commits_tab().diff_scroll, 0);
        assert!(state.commits_tab().expanded_sha.is_none());
    }
}
