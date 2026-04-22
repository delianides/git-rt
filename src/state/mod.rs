use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::git::FileEntry;

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
    pub url: String,
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
    /// Whether the help popup is currently shown
    help_visible: bool,
    /// True when a worker `Recompute` request has been sent and no
    /// `Status` response has come back yet. Lets the UI surface a subtle
    /// indicator without blocking input.
    is_computing: bool,
    /// PR widget state
    pr_state: PrState,
    /// When set, the pane border should flash until this instant.
    border_flash_until: Option<Instant>,
    /// Cached merge base commit id (None when on the base branch or can't compute)
    merge_base: Option<gix::ObjectId>,
    /// Resolved base branch name
    base_branch: String,
    /// Viewport offset into the file list. Persisted across renders so
    /// ratatui's `List::scroll_padding` can maintain sticky scroll behavior.
    /// Mutated in place by the widget during `render_stateful_widget` and
    /// read back by the render function.
    scroll_offset: usize,
    /// False until the first `update_files` call completes. Prevents the
    /// initial git-status snapshot (startup or post-worktree-switch) from
    /// flashing every row, since there's nothing to meaningfully compare
    /// against.
    initial_seed_done: bool,
}

impl AppState {
    pub fn new(files: Vec<FileEntry>, flash_duration: Duration, branch: String) -> Self {
        let now = Instant::now();
        Self {
            files,
            selected: 0,
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
            help_visible: false,
            is_computing: false,
            pr_state: PrState::default(),
            border_flash_until: None,
            merge_base: None,
            base_branch: String::new(),
            scroll_offset: 0,
            initial_seed_done: false,
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

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
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

    pub fn merge_base(&self) -> Option<gix::ObjectId> {
        self.merge_base
    }

    pub fn base_branch(&self) -> &str {
        &self.base_branch
    }

    pub fn set_merge_base(&mut self, mb: Option<gix::ObjectId>, base_branch: String) {
        self.merge_base = mb;
        self.base_branch = base_branch;
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

    // -- Help overlay --

    /// Returns true if the help popup is currently visible
    pub fn is_help_visible(&self) -> bool {
        self.help_visible
    }

    /// Show the help popup.
    pub fn show_help(&mut self) {
        self.help_visible = true;
    }

    /// Hide the help popup
    pub fn hide_help(&mut self) {
        self.help_visible = false;
    }

    /// True when a recompute is in flight.
    pub fn is_computing(&self) -> bool {
        self.is_computing
    }

    /// Set the in-flight indicator. The event loop sets this true when sending
    /// `Request::Recompute` and clears it on `Response::Status` / `Error`.
    pub fn set_computing(&mut self, value: bool) {
        self.is_computing = value;
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

    // -- Navigation --

    pub fn select_next(&mut self) {
        if !self.files.is_empty() {
            self.selected = (self.selected + 1).min(self.files.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
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
        self.is_computing = false;
        self.pr_state = PrState::default();
        // Activate border flash for visual feedback on switch
        self.border_flash_until = Some(Instant::now() + self.flash_duration);
        self.merge_base = None;
        self.base_branch.clear();
        self.scroll_offset = 0;
    }

    /// Update the file list from a fresh git status computation.
    /// Preserves selection position and expanded state where possible.
    pub fn update_files(&mut self, new_files: Vec<FileEntry>) {
        // Try to preserve the selected file by path
        let selected_path = self.selected_path();

        let now = Instant::now();

        if self.initial_seed_done {
            // Build a snapshot of old file stats for flash detection
            let old_stats: HashMap<&str, (usize, usize)> = self
                .files
                .iter()
                .map(|f| (f.path.as_str(), (f.insertions, f.deletions)))
                .collect();

            // Detect changed files and record flash times
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
        } else {
            // First populate after construction or worktree switch — treat
            // as baseline, not a set of changes. Future calls will flash
            // real diffs against this baseline.
            self.initial_seed_done = true;
        }

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

        // `scroll_offset` is intentionally not reset here. ratatui's
        // `get_items_bounds` clamps any stale offset against the new list
        // length on the next render, and the render path writes the corrected
        // value back via `set_scroll_offset`. Resetting to 0 here would cause
        // a one-frame viewport jump on every FS recompute.
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
    fn test_accessors() {
        let files = vec![make_entry("a.rs", 1, 0), make_entry("b.rs", 2, 1)];
        let state = AppState::new(files, Duration::from_millis(600), "main".to_string());

        assert_eq!(state.files().len(), 2);
        assert_eq!(state.files()[0].path, "a.rs");
        assert_eq!(state.selected_index(), 0);
        assert_eq!(state.selected_path(), Some("a.rs".to_string()));
        assert_eq!(state.refresh_count(), 0);
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
    fn test_update_increments_refresh_count() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.refresh_count(), 0);

        state.update_files(vec![make_entry("a.rs", 1, 0)]);
        assert_eq!(state.refresh_count(), 1);

        state.update_files(vec![]);
        assert_eq!(state.refresh_count(), 2);
    }

    #[test]
    fn test_flash_on_changed_stats() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        // Seed the baseline via the first update_files (matches production)
        state.update_files(vec![make_entry("a.rs", 1, 0)]);
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
        let mut state = AppState::new(vec![], Duration::from_millis(1), "main".to_string()); // 1ms flash
                                                                                             // Seed the baseline
        state.update_files(vec![make_entry("a.rs", 1, 0)]);

        // Change triggers a flash
        state.update_files(vec![make_entry("a.rs", 5, 2)]);
        // Sleep just past the flash duration
        std::thread::sleep(Duration::from_millis(5));
        assert!(!state.is_flashing("a.rs"));
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
    fn test_help_visible_default_false() {
        let s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!s.is_help_visible());
    }

    #[test]
    fn test_show_hide_help() {
        let mut s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        s.show_help();
        assert!(s.is_help_visible());
        s.hide_help();
        assert!(!s.is_help_visible());
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
            url: String::new(),
        });
        assert!(!state.pr_state().loading);
        assert_eq!(state.pr_state().info.as_ref().unwrap().number, 42);
    }

    #[test]
    fn test_reset_clears_pr() {
        let files = vec![make_entry("a.rs", 1, 0)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
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
            url: String::new(),
        });
        state.reset_for_switch(vec![], "main".to_string(), "r".to_string(), "w".to_string());
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
    fn test_merge_base_default_none() {
        let state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(state.merge_base().is_none());
        assert_eq!(state.base_branch(), "");
    }

    #[test]
    fn test_set_merge_base() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "feature".to_string());
        let fake_id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        state.set_merge_base(Some(fake_id), "main".to_string());
        assert_eq!(state.merge_base(), Some(fake_id));
        assert_eq!(state.base_branch(), "main");
    }

    #[test]
    fn test_reset_for_switch_clears_merge_base() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "feature".to_string());
        let fake_id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        state.set_merge_base(Some(fake_id), "main".to_string());

        state.reset_for_switch(
            vec![],
            "other".to_string(),
            "r".to_string(),
            "w".to_string(),
        );
        assert!(state.merge_base().is_none());
        assert_eq!(state.base_branch(), "");
    }

    #[test]
    fn test_reset_for_switch() {
        let files = vec![make_entry("a.rs", 1, 0), make_entry("b.rs", 2, 1)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.select_next(); // select b.rs
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
        assert_eq!(state.files().len(), 1);
        assert_eq!(state.files()[0].path, "c.rs");
        assert_eq!(state.branch(), "feature");
        assert_eq!(state.repo_name(), "new-repo");
        assert_eq!(state.worktree_name(), "new-wt");
        assert_eq!(state.refresh_count(), 0);
    }

    #[test]
    fn is_computing_defaults_false_and_is_set_clear() {
        let mut s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert!(!s.is_computing());
        s.set_computing(true);
        assert!(s.is_computing());
        s.set_computing(false);
        assert!(!s.is_computing());
    }

    #[test]
    fn update_files_initial_seed_does_not_flash() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.update_files(vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 2, 1),
            make_entry("c.rs", 0, 3),
        ]);
        assert!(!state.is_flashing("a.rs"));
        assert!(!state.is_flashing("b.rs"));
        assert!(!state.is_flashing("c.rs"));
    }

    #[test]
    fn update_files_after_seed_flashes_changed_numstat() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.update_files(vec![make_entry("a.rs", 1, 0)]); // seed
        state.update_files(vec![make_entry("a.rs", 5, 2)]); // stats change
        assert!(state.is_flashing("a.rs"));
    }

    #[test]
    fn update_files_after_seed_flashes_new_file() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.update_files(vec![make_entry("a.rs", 1, 0)]); // seed
        state.update_files(vec![
            make_entry("a.rs", 1, 0), // unchanged
            make_entry("b.rs", 2, 1), // new
        ]);
        assert!(!state.is_flashing("a.rs"));
        assert!(state.is_flashing("b.rs"));
    }

    #[test]
    fn test_scroll_offset_default_and_roundtrip() {
        let mut state = AppState::new(
            vec![make_entry("a.rs", 1, 0)],
            Duration::from_millis(600),
            "main".to_string(),
        );
        assert_eq!(state.scroll_offset(), 0);

        state.set_scroll_offset(17);
        assert_eq!(state.scroll_offset(), 17);
    }

    #[test]
    fn test_reset_for_switch_clears_scroll_offset() {
        let mut state = AppState::new(
            vec![make_entry("a.rs", 1, 0)],
            Duration::from_millis(600),
            "main".to_string(),
        );
        state.set_scroll_offset(42);
        assert_eq!(state.scroll_offset(), 42);

        state.reset_for_switch(
            vec![make_entry("b.rs", 1, 0)],
            "other".to_string(),
            "repo".to_string(),
            "wt".to_string(),
        );
        assert_eq!(state.scroll_offset(), 0);
    }
}
