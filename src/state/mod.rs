use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::git::{ChangeGroup, FileDiff, FileEntry};
use crate::ui::switch_dialog::SwitchDialog;
use crate::ui::tree::{build_normal_rows, build_visible_rows, RowId, VisibleRow};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    /// Single flat list of every changed file path.
    Condensed,
    /// Files arranged as a directory tree.
    Tree,
    /// Files split into collapsible status-group sections.
    Normal,
}

/// The direction of the change that triggered a file-row flash, used to pick
/// the flash color: a net gain in lines (or no net change) flashes as an
/// addition, a net loss flashes as a deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashKind {
    /// The change added at least as many lines as it removed.
    Added,
    /// The change removed more lines than it added.
    Removed,
}

#[derive(Debug, Clone)]
struct SelectionSnapshot {
    row_id: Option<RowId>,
    file_path: Option<String>,
}

#[derive(Debug, Clone)]
struct ExpandedDiff {
    path: String,
    insertions: usize,
    deletions: usize,
    diff: FileDiff,
}

/// The application's view model — what the UI renders from
pub struct AppState {
    /// All changed files in the repo
    files: Vec<FileEntry>,
    /// Currently selected index in the active visible rows
    selected: usize,
    /// Current list presentation mode
    view_mode: ViewMode,
    /// Expanded directories while in tree mode
    expanded_dirs: BTreeSet<String>,
    /// Collapsed status groups while in Normal mode. Not persisted; reset
    /// on launch and on worktree switch.
    collapsed_groups: HashSet<ChangeGroup>,
    /// Stable identity for the selected row when one has been established
    selected_row_id: Option<RowId>,
    /// Number of times the file list has been refreshed
    refresh_count: usize,
    /// When the app started (for computing "last updated N seconds ago")
    start_time: Instant,
    /// When the last refresh happened
    last_refresh: Instant,
    /// Tracks when each file last changed (for the flash effect) and the
    /// direction of that change (for the flash color).
    flash_times: HashMap<String, (Instant, FlashKind)>,
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
    /// Last-rendered height (rows) of the file-list viewport. Written by the
    /// UI each frame; read by half/full-page navigation.
    list_viewport_height: usize,
    /// False until the first `update_files` call completes. Prevents the
    /// initial git-status snapshot (startup or post-worktree-switch) from
    /// flashing every row, since there's nothing to meaningfully compare
    /// against.
    initial_seed_done: bool,
    /// Diff for the currently expanded file (if any)
    current_diff: Option<ExpandedDiff>,
    /// Vertical scroll offset inside the diff overlay, in lines
    diff_scroll: usize,
    /// Last-rendered height (rows) of the diff overlay viewport. Written by the
    /// UI each frame; read by half/full-page scrolling and jump-to-bottom.
    diff_viewport_height: usize,
    /// Whether the diff overlay is currently shown
    diff_overlay_visible: bool,
    /// Monotonically-increasing counter. Bumped on each `Request::Diff`
    /// so stale `Response::Diff` messages whose token doesn't match the
    /// current one get dropped by the app event loop.
    pending_diff_token: u64,
    /// The switch-worktree dialog, when open.
    switch_dialog: Option<SwitchDialog>,
}

impl AppState {
    pub fn new(files: Vec<FileEntry>, flash_duration: Duration, branch: String) -> Self {
        let now = Instant::now();
        Self {
            files,
            selected: 0,
            view_mode: ViewMode::Condensed,
            expanded_dirs: BTreeSet::new(),
            collapsed_groups: HashSet::new(),
            selected_row_id: None,
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
            list_viewport_height: 0,
            initial_seed_done: false,
            current_diff: None,
            diff_scroll: 0,
            diff_viewport_height: 0,
            diff_overlay_visible: false,
            pending_diff_token: 0,
            switch_dialog: None,
        }
    }

    // -- Accessors --

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    /// True once the first `update_files` call has completed. Used by the
    /// Normal renderer to avoid showing the no-base error before the first
    /// git-status snapshot arrives.
    pub fn initial_seed_done(&self) -> bool {
        self.initial_seed_done
    }

    pub fn expanded_dirs(&self) -> &BTreeSet<String> {
        &self.expanded_dirs
    }

    pub fn selected_path(&self) -> Option<String> {
        self.selected_file_path()
    }

    pub fn selected_file_path(&self) -> Option<String> {
        self.selection_snapshot().file_path
    }

    pub fn visible_rows(&self) -> Vec<VisibleRow> {
        self.rebuild_visible_rows()
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
        self.flash_kind(path).is_some()
    }

    /// Returns the flash direction for the file at `path` if it is currently
    /// flashing, or `None` if it is not flashing (or the flash has expired).
    pub fn flash_kind(&self, path: &str) -> Option<FlashKind> {
        self.flash_times
            .get(path)
            .filter(|(t, _)| t.elapsed() < self.flash_duration)
            .map(|(_, kind)| *kind)
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

    /// Show the help popup. Also hides the diff overlay to enforce the
    /// "only one overlay at a time" rule.
    pub fn show_help(&mut self) {
        self.help_visible = true;
        self.diff_overlay_visible = false;
        self.diff_scroll = 0;
    }

    /// Hide the help popup
    pub fn hide_help(&mut self) {
        self.help_visible = false;
    }

    // -- Diff overlay --

    /// Returns true if the diff overlay is currently visible.
    pub fn is_diff_overlay_visible(&self) -> bool {
        self.diff_overlay_visible
    }

    /// Show the diff overlay.
    pub fn show_diff_overlay(&mut self) {
        self.diff_overlay_visible = true;
    }

    /// Hide the diff overlay and reset diff scroll.
    pub fn hide_diff_overlay(&mut self) {
        self.diff_overlay_visible = false;
        self.diff_scroll = 0;
    }

    // -- Switch dialog --

    /// True when the switch-worktree dialog is open.
    pub fn is_switch_dialog_visible(&self) -> bool {
        self.switch_dialog.is_some()
    }

    /// Open the switch dialog with the given dialog state.
    pub fn show_switch_dialog(&mut self, dialog: SwitchDialog) {
        self.switch_dialog = Some(dialog);
    }

    /// Close the switch dialog.
    pub fn hide_switch_dialog(&mut self) {
        self.switch_dialog = None;
    }

    /// Borrow the dialog mutably (for key handling).
    pub fn switch_dialog_mut(&mut self) -> Option<&mut SwitchDialog> {
        self.switch_dialog.as_mut()
    }

    /// Borrow the dialog immutably (for rendering).
    pub fn switch_dialog(&self) -> Option<&SwitchDialog> {
        self.switch_dialog.as_ref()
    }

    /// Currently expanded diff, if any.
    pub fn expanded_diff(&self) -> Option<&FileDiff> {
        self.current_diff.as_ref().map(|diff| &diff.diff)
    }

    /// File path associated with the currently expanded diff.
    pub fn expanded_diff_path(&self) -> Option<&str> {
        self.current_diff.as_ref().map(|diff| diff.path.as_str())
    }

    /// Insertions/deletions associated with the currently expanded diff.
    pub fn expanded_diff_stats(&self) -> Option<(usize, usize)> {
        self.current_diff
            .as_ref()
            .map(|diff| (diff.insertions, diff.deletions))
    }

    /// Current scroll offset inside the diff overlay.
    pub fn diff_scroll(&self) -> usize {
        self.diff_scroll
    }

    /// Scroll the diff down by one line.
    pub fn scroll_diff_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(1);
    }

    /// Scroll the diff up by one line.
    pub fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(1);
    }

    /// Last-rendered diff viewport height in rows (0 until first render).
    pub fn diff_viewport_height(&self) -> usize {
        self.diff_viewport_height
    }

    /// Record the diff viewport height. Called by the UI each frame.
    pub fn set_diff_viewport_height(&mut self, height: usize) {
        self.diff_viewport_height = height;
    }

    /// Logical line count of the current diff: one header line per hunk plus
    /// each hunk's lines. Matches what the overlay renders before wrapping, so
    /// jump-to-bottom is approximate when lines wrap. Zero when no diff is set.
    pub fn diff_total_lines(&self) -> usize {
        self.current_diff
            .as_ref()
            .map(|d| d.diff.hunks.iter().map(|h| 1 + h.lines.len()).sum())
            .unwrap_or(0)
    }

    /// Maximum valid scroll offset: keeps at least one screen of content.
    fn diff_max_scroll(&self) -> usize {
        self.diff_total_lines()
            .saturating_sub(self.diff_viewport_height)
    }

    /// Scroll the diff to the very top.
    pub fn scroll_diff_to_top(&mut self) {
        self.diff_scroll = 0;
    }

    /// Scroll the diff so the last lines are visible.
    pub fn scroll_diff_to_bottom(&mut self) {
        self.diff_scroll = self.diff_max_scroll();
    }

    /// Scroll the diff by `delta` lines (negative = up), clamped to
    /// `[0, diff_max_scroll]`. Used for half/full-page scrolling.
    pub fn scroll_diff_page(&mut self, delta: isize) {
        let target = (self.diff_scroll as isize + delta).max(0) as usize;
        self.diff_scroll = target.min(self.diff_max_scroll());
    }

    /// Bump the pending-diff token and return the new value. Stamp this
    /// token on the outgoing `Request::Diff` so stale responses can be
    /// filtered out.
    pub fn advance_pending_diff_token(&mut self) -> u64 {
        self.pending_diff_token = self.pending_diff_token.wrapping_add(1);
        self.pending_diff_token
    }

    /// Current pending diff token. `Response::Diff` results with a different
    /// token must be dropped by the caller.
    pub fn pending_diff_token(&self) -> u64 {
        self.pending_diff_token
    }

    /// Set the current diff (used when a Response::Diff arrives whose
    /// token matches the pending token). Also resets scroll to top.
    pub fn set_expanded_diff(&mut self, path: String, diff: FileDiff) {
        let (insertions, deletions) = self
            .files
            .iter()
            .find(|file| file.path == path)
            .map(|file| (file.insertions, file.deletions))
            .unwrap_or((0, 0));
        self.current_diff = Some(ExpandedDiff {
            path,
            insertions,
            deletions,
            diff,
        });
        self.diff_scroll = 0;
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
        let rows = self.rebuild_visible_rows();
        if !rows.is_empty() {
            self.set_selection_from_rows(&rows, self.selected + 1);
        }
    }

    pub fn select_previous(&mut self) {
        let rows = self.rebuild_visible_rows();
        self.set_selection_from_rows(&rows, self.selected.saturating_sub(1));
    }

    /// Last-rendered file-list viewport height in rows (0 until first render).
    pub fn list_viewport_height(&self) -> usize {
        self.list_viewport_height
    }

    /// Record the file-list viewport height (drawable item rows, after the
    /// pane inset is applied). Called by the UI each frame.
    pub fn set_list_viewport_height(&mut self, height: usize) {
        self.list_viewport_height = height;
    }

    /// Move the selection to the first visible row.
    pub fn select_first(&mut self) {
        let rows = self.rebuild_visible_rows();
        self.set_selection_from_rows(&rows, 0);
    }

    /// Move the selection to the last visible row.
    pub fn select_last(&mut self) {
        let rows = self.rebuild_visible_rows();
        let last = rows.len().saturating_sub(1);
        self.set_selection_from_rows(&rows, last);
    }

    /// Move the selection by `delta` rows (negative = up), clamped to the
    /// visible range. No-op on an empty list. Used for half/full-page jumps.
    pub fn select_page(&mut self, delta: isize) {
        let rows = self.rebuild_visible_rows();
        if rows.is_empty() {
            return;
        }
        let target = (self.selected as isize + delta).max(0) as usize;
        self.set_selection_from_rows(&rows, target);
    }

    pub fn cycle_view_mode(&mut self) {
        let selection = self.selection_snapshot();

        self.view_mode = match self.view_mode {
            ViewMode::Normal => ViewMode::Condensed,
            ViewMode::Condensed => ViewMode::Tree,
            ViewMode::Tree => ViewMode::Normal,
        };
        self.scroll_offset = 0;

        if self.view_mode == ViewMode::Tree {
            if let Some(path) = selection.file_path.as_deref() {
                self.expand_ancestors_for_file(path);
            }
        }

        self.restore_selection(selection);
    }

    /// Set the view mode directly (used to apply the configured default).
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
    }

    pub fn toggle_selected_directory(&mut self) -> bool {
        let rows = self.rebuild_visible_rows();
        let Some(RowId::Directory(path)) =
            Self::selected_row_from_rows(&rows, self.selected).map(|row| row.id().clone())
        else {
            return false;
        };

        if !self.expanded_dirs.insert(path.clone()) {
            self.expanded_dirs.remove(&path);
        }

        let rows = self.rebuild_visible_rows();
        let selected_directory = RowId::Directory(path);
        if let Some(index) = rows.iter().position(|row| row.id() == &selected_directory) {
            self.set_selection_from_rows(&rows, index);
        } else {
            self.set_selection_from_rows(&rows, self.selected);
        }
        true
    }

    /// In Normal mode, toggle the collapsed state of the status group whose
    /// header is currently selected. Returns `true` if a header was toggled.
    pub fn toggle_selected_group(&mut self) -> bool {
        let rows = self.rebuild_visible_rows();
        let Some(RowId::Group(group)) =
            Self::selected_row_from_rows(&rows, self.selected).map(|row| row.id().clone())
        else {
            return false;
        };

        if !self.collapsed_groups.insert(group) {
            self.collapsed_groups.remove(&group);
        }

        let rows = self.rebuild_visible_rows();
        let selected_header = RowId::Group(group);
        if let Some(index) = rows.iter().position(|row| row.id() == &selected_header) {
            self.set_selection_from_rows(&rows, index);
        } else {
            self.set_selection_from_rows(&rows, self.selected);
        }
        true
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
        self.expanded_dirs.clear();
        self.collapsed_groups.clear();
        self.selected_row_id = None;
        self.refresh_count = 0;
        self.last_refresh = Instant::now();
        self.flash_times.clear();
        self.initial_seed_done = false;
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
        self.current_diff = None;
        self.diff_overlay_visible = false;
        self.diff_scroll = 0;
        self.diff_viewport_height = 0;
        self.switch_dialog = None;
        self.pending_diff_token = self.pending_diff_token.wrapping_add(1);
    }

    /// Update the file list from a fresh git status computation.
    /// Preserves selection position and expanded state where possible.
    #[tracing::instrument(name = "state.update_files", skip_all, fields(n = new_files.len()))]
    pub fn update_files(&mut self, new_files: Vec<FileEntry>) {
        let selection = self.selection_snapshot();

        let now = Instant::now();

        if self.initial_seed_done {
            // Build a snapshot of old file stats for flash detection
            let old_stats: HashMap<&str, (usize, usize)> = self
                .files
                .iter()
                .map(|f| (f.path.as_str(), (f.insertions, f.deletions)))
                .collect();

            // Detect changed files and record flash times. The flash color
            // reflects the net line delta of the change: a file new to the
            // list, or one whose net (insertions - deletions) grew or held
            // steady, flashes as an addition; one whose net shrank flashes as
            // a deletion.
            for file in &new_files {
                let (old_ins, old_del) =
                    old_stats.get(file.path.as_str()).copied().unwrap_or((0, 0));
                let changed = match old_stats.get(file.path.as_str()) {
                    Some(&(o_ins, o_del)) => o_ins != file.insertions || o_del != file.deletions,
                    None => true, // new file
                };
                if changed {
                    let old_net = old_ins as isize - old_del as isize;
                    let new_net = file.insertions as isize - file.deletions as isize;
                    let kind = if new_net >= old_net {
                        FlashKind::Added
                    } else {
                        FlashKind::Removed
                    };
                    self.flash_times.insert(file.path.clone(), (now, kind));
                }
            }

            // Clean up expired flash times
            self.flash_times
                .retain(|_, (t, _)| t.elapsed() < self.flash_duration);
        } else {
            // First populate after construction or worktree switch — treat
            // as baseline, not a set of changes. Future calls will flash
            // real diffs against this baseline.
            self.initial_seed_done = true;
        }

        self.files = new_files;
        self.refresh_count += 1;
        self.last_refresh = now;

        if self.view_mode == ViewMode::Tree {
            if let Some(path) = selection.file_path.as_deref() {
                self.expand_ancestors_for_file(path);
            }
        }

        self.restore_selection(selection);

        // `scroll_offset` is intentionally not reset here. ratatui's
        // `get_items_bounds` clamps any stale offset against the new list
        // length on the next render, and the render path writes the corrected
        // value back via `set_scroll_offset`. Resetting to 0 here would cause
        // a one-frame viewport jump on every FS recompute.
    }

    #[tracing::instrument(name = "state.rebuild_visible_rows", skip_all)]
    fn rebuild_visible_rows(&self) -> Vec<VisibleRow> {
        match self.view_mode {
            ViewMode::Condensed => self.files.iter().map(condensed_row_from_file).collect(),
            ViewMode::Tree => build_visible_rows(&self.files, &self.expanded_dirs),
            ViewMode::Normal => build_normal_rows(&self.files, &self.collapsed_groups),
        }
    }

    fn selection_snapshot(&self) -> SelectionSnapshot {
        let rows = self.rebuild_visible_rows();
        Self::selection_snapshot_from_rows(&rows, self.selected)
    }

    fn selection_snapshot_from_rows(rows: &[VisibleRow], selected: usize) -> SelectionSnapshot {
        let row = Self::selected_row_from_rows(rows, selected);
        SelectionSnapshot {
            row_id: row.map(|row| row.id().clone()),
            file_path: row.and_then(|row| match row.id() {
                RowId::File(path) => Some(path.clone()),
                RowId::Directory(_) | RowId::Group(_) => None,
            }),
        }
    }

    fn selected_row_from_rows(rows: &[VisibleRow], selected: usize) -> Option<&VisibleRow> {
        rows.get(selected)
    }

    fn set_selection_from_rows(&mut self, rows: &[VisibleRow], selected: usize) {
        if rows.is_empty() {
            self.selected = 0;
            self.selected_row_id = None;
            return;
        }

        self.selected = selected.min(rows.len() - 1);
        self.selected_row_id = rows.get(self.selected).map(|row| row.id().clone());
    }

    fn restore_selection(&mut self, selection: SelectionSnapshot) {
        match self.view_mode {
            ViewMode::Condensed => self.restore_condensed_selection(selection.file_path),
            ViewMode::Tree => self.restore_selection_by_id_or_path(selection),
            ViewMode::Normal => self.restore_selection_by_id_or_path(selection),
        }
    }

    fn restore_condensed_selection(&mut self, selected_file_path: Option<String>) {
        if self.files.is_empty() {
            self.selected = 0;
            self.selected_row_id = None;
            return;
        }

        if let Some(path) = selected_file_path {
            self.selected = self
                .files
                .iter()
                .position(|file| file.path == path)
                .unwrap_or(self.selected.min(self.files.len() - 1));
        } else {
            self.selected = self.selected.min(self.files.len() - 1);
        }

        self.selected_row_id = self
            .files
            .get(self.selected)
            .map(|file| RowId::File(file.path.clone()));
    }

    /// Restore selection after a rebuild by stable row id, then by file path, then by clamped index. Shared by Tree and Normal modes.
    fn restore_selection_by_id_or_path(&mut self, selection: SelectionSnapshot) {
        let rows = self.rebuild_visible_rows();

        if let Some(selected_row_id) = selection.row_id.as_ref() {
            if let Some(index) = rows.iter().position(|row| row.id() == selected_row_id) {
                self.set_selection_from_rows(&rows, index);
                return;
            }
        }

        if let Some(selected_file_path) = selection.file_path.as_ref() {
            let file_id = RowId::File(selected_file_path.clone());
            if let Some(index) = rows.iter().position(|row| row.id() == &file_id) {
                self.set_selection_from_rows(&rows, index);
                return;
            }
        }

        self.set_selection_from_rows(&rows, self.selected);
    }

    fn expand_ancestors_for_file(&mut self, path: &str) {
        let mut parts: Vec<&str> = path.split('/').collect();
        if parts.len() <= 1 {
            return;
        }

        parts.pop();

        let mut current = String::new();
        for segment in parts {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            self.expanded_dirs.insert(current.clone());
        }
    }
}

fn condensed_row_from_file(file: &FileEntry) -> VisibleRow {
    VisibleRow::File {
        id: RowId::File(file.path.clone()),
        depth: 0,
        label: file.path.clone(),
        file: file.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{ChangeGroup, FileStatus};

    fn make_entry(path: &str, ins: usize, del: usize) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: ins,
            deletions: del,
            group: ChangeGroup::Changes,
        }
    }

    fn grouped_entry(path: &str, group: ChangeGroup) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 0,
            group,
        }
    }

    #[test]
    fn test_normal_mode_builds_header_rows() {
        let files = vec![
            grouped_entry("a.rs", ChangeGroup::Changes),
            grouped_entry("b.rs", ChangeGroup::Committed),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        let rows = state.visible_rows();
        assert_eq!(rows.len(), 4); // 2 headers + 2 files
        assert!(rows[0].is_header());
    }

    #[test]
    fn test_toggle_selected_group_collapses_and_keeps_header_selected() {
        let files = vec![grouped_entry("a.rs", ChangeGroup::Changes)];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        // Row 0 is the "Changes" header.
        assert!(state.toggle_selected_group());
        let rows = state.visible_rows();
        assert_eq!(rows.len(), 1); // file hidden, header remains
        assert_eq!(rows[0].header_collapsed(), Some(true));
        assert_eq!(state.selected_index(), 0); // still on the header
                                               // Toggling again expands it.
        assert!(state.toggle_selected_group());
        assert_eq!(state.visible_rows().len(), 2);
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
    fn test_cycle_view_mode_condensed_to_tree_to_normal() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.view_mode(), ViewMode::Condensed);
        state.cycle_view_mode();
        assert_eq!(state.view_mode(), ViewMode::Tree);
        state.cycle_view_mode();
        assert_eq!(state.view_mode(), ViewMode::Normal);
    }

    #[test]
    fn test_cycle_view_mode_three_way() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.set_view_mode(ViewMode::Normal);
        assert_eq!(state.view_mode(), ViewMode::Normal);
        state.cycle_view_mode();
        assert_eq!(state.view_mode(), ViewMode::Condensed);
        state.cycle_view_mode();
        assert_eq!(state.view_mode(), ViewMode::Tree);
        state.cycle_view_mode();
        assert_eq!(state.view_mode(), ViewMode::Normal);
    }

    #[test]
    fn test_tree_mode_auto_expands_ancestors_for_selected_file() {
        let files = vec![
            make_entry("src/ui/header.rs", 2, 0),
            make_entry("src/ui/mod.rs", 1, 0),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        assert!(state.expanded_dirs().contains("src"));
        assert!(state.expanded_dirs().contains("src/ui"));
        assert_eq!(
            state.selected_file_path().as_deref(),
            Some("src/ui/header.rs")
        );
    }

    #[test]
    fn test_toggle_selected_directory_collapses_visible_children() {
        let files = vec![
            make_entry("src/ui/header.rs", 2, 0),
            make_entry("src/ui/mod.rs", 1, 0),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        let expanded_labels = state
            .visible_rows()
            .into_iter()
            .map(|row| row.label().to_string())
            .collect::<Vec<_>>();
        assert_eq!(expanded_labels, vec!["src/ui/", "header.rs", "mod.rs"]);

        state.select_previous();
        assert!(state.toggle_selected_directory());
        assert!(!state.expanded_dirs().contains("src/ui"));

        let collapsed_labels = state
            .visible_rows()
            .into_iter()
            .map(|row| row.label().to_string())
            .collect::<Vec<_>>();
        assert_eq!(collapsed_labels, vec!["src/ui/"]);
        assert!(state.selected_file_path().is_none());
    }

    #[test]
    fn test_tree_mode_update_preserves_selected_file_by_path() {
        let files = vec![
            make_entry("src/ui/header.rs", 2, 0),
            make_entry("src/ui/mod.rs", 1, 0),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.cycle_view_mode();

        let updated_files = vec![
            make_entry("src/ui/footer.rs", 3, 0),
            make_entry("src/ui/header.rs", 5, 1),
            make_entry("src/ui/mod.rs", 1, 0),
        ];
        state.update_files(updated_files);

        assert_eq!(
            state.selected_file_path().as_deref(),
            Some("src/ui/header.rs")
        );
        assert!(state.expanded_dirs().contains("src"));
        assert!(state.expanded_dirs().contains("src/ui"));
        assert_eq!(state.selected_index(), 2);
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
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        // Seed the baseline.
        state.update_files(vec![make_entry("a.rs", 1, 0)]);

        // Update with same stats — must not flash, exercising the changed=false
        // branch inside the gate (not the initial-seed branch).
        state.update_files(vec![make_entry("a.rs", 1, 0)]);
        assert!(!state.is_flashing("a.rs"));
    }

    #[test]
    fn test_flash_expires() {
        // 1ms flash duration so the sleep below reliably outruns it.
        let mut state = AppState::new(vec![], Duration::from_millis(1), "main".to_string());
        // Seed the baseline.
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

        state.set_repo_name("perch".to_string());
        state.set_worktree_name("perch".to_string());
        state.set_head_info("abc1234".to_string(), "fix: some bug".to_string());
        state.set_stash_count(3);
        state.set_ahead_behind(Some((2, 1)));
        state.set_repo_state(Some("REBASING".to_string()));

        assert_eq!(state.repo_name(), "perch");
        assert_eq!(state.worktree_name(), "perch");
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
    fn flash_kind_reflects_net_line_delta() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        // Seed: a is +5, b is +5, c is +5.
        state.update_files(vec![
            make_entry("a.rs", 5, 0),
            make_entry("b.rs", 5, 0),
            make_entry("c.rs", 5, 0),
        ]);
        // a gains lines (net up), b loses lines (net down), c stays net-equal
        // (added 2, removed 2), and d is brand new.
        state.update_files(vec![
            make_entry("a.rs", 8, 0), // net 5 -> 8: added
            make_entry("b.rs", 5, 3), // net 5 -> 2: removed
            make_entry("c.rs", 7, 2), // net 5 -> 5: tie -> added
            make_entry("d.rs", 0, 4), // new file, net 0 - 4 vs 0: removed
        ]);
        assert_eq!(state.flash_kind("a.rs"), Some(FlashKind::Added));
        assert_eq!(state.flash_kind("b.rs"), Some(FlashKind::Removed));
        assert_eq!(state.flash_kind("c.rs"), Some(FlashKind::Added));
        assert_eq!(state.flash_kind("d.rs"), Some(FlashKind::Removed));
    }

    #[test]
    fn flash_kind_none_when_not_flashing() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.update_files(vec![make_entry("a.rs", 1, 0)]); // seed, no flash
        assert_eq!(state.flash_kind("a.rs"), None);
        assert_eq!(state.flash_kind("missing.rs"), None);
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

    #[test]
    fn reset_for_switch_clears_seed_flag() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        // Seed the baseline
        state.update_files(vec![make_entry("a.rs", 1, 0)]);

        // Switch worktree — should force the next update_files to be a new baseline
        state.reset_for_switch(
            Vec::new(),
            "feat/foo".to_string(),
            "repo".to_string(),
            "wt".to_string(),
        );

        // First post-switch update must not flash any row
        state.update_files(vec![make_entry("x.rs", 5, 2), make_entry("y.rs", 0, 1)]);
        assert!(!state.is_flashing("x.rs"));
        assert!(!state.is_flashing("y.rs"));
    }

    #[test]
    fn post_switch_change_on_clean_branch_flashes() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        // Initial seed (empty state)
        state.update_files(vec![]);

        // Switch to a clean branch
        state.reset_for_switch(
            Vec::new(),
            "feat/foo".to_string(),
            "repo".to_string(),
            "wt".to_string(),
        );

        // First post-switch update: empty (the new branch is clean)
        state.update_files(vec![]);

        // User edits a file on the new branch
        state.update_files(vec![make_entry("new.rs", 3, 1)]);

        // That file must flash — this is the case that rules out the
        // "empty-list" shortcut alternative design.
        assert!(state.is_flashing("new.rs"));
    }

    fn make_state() -> AppState {
        AppState::new(vec![], Duration::from_millis(600), "main".to_string())
    }

    #[test]
    fn test_diff_total_lines_no_diff_is_zero() {
        let state = make_state();
        assert_eq!(state.diff_total_lines(), 0);
    }

    #[test]
    fn test_overlay_show_hide() {
        let mut state = make_state();
        assert!(!state.is_diff_overlay_visible());
        state.show_diff_overlay();
        assert!(state.is_diff_overlay_visible());
        state.hide_diff_overlay();
        assert!(!state.is_diff_overlay_visible());
    }

    #[test]
    fn test_hide_overlay_resets_scroll() {
        let mut state = make_state();
        state.show_diff_overlay();
        state.scroll_diff_down();
        state.scroll_diff_down();
        assert_eq!(state.diff_scroll(), 2);
        state.hide_diff_overlay();
        assert_eq!(state.diff_scroll(), 0);
    }

    #[test]
    fn test_scroll_diff_saturates_at_zero() {
        let mut state = make_state();
        state.scroll_diff_up();
        state.scroll_diff_up();
        assert_eq!(state.diff_scroll(), 0, "should not underflow");
    }

    #[test]
    fn test_scroll_diff_up_and_down() {
        let mut state = make_state();
        state.scroll_diff_down();
        state.scroll_diff_down();
        state.scroll_diff_down();
        assert_eq!(state.diff_scroll(), 3);
        state.scroll_diff_up();
        assert_eq!(state.diff_scroll(), 2);
    }

    #[test]
    fn test_pending_diff_token_increments_monotonically() {
        let mut state = make_state();
        let t0 = state.pending_diff_token();
        let t1 = state.advance_pending_diff_token();
        let t2 = state.advance_pending_diff_token();
        assert!(t1 > t0);
        assert!(t2 > t1);
        assert_eq!(state.pending_diff_token(), t2);
    }

    #[test]
    fn test_show_help_hides_diff_overlay() {
        let mut state = make_state();
        state.show_diff_overlay();
        state.scroll_diff_down();
        assert!(state.is_diff_overlay_visible());
        state.show_help();
        assert!(
            !state.is_diff_overlay_visible(),
            "help should close diff overlay"
        );
        assert_eq!(state.diff_scroll(), 0, "help should reset diff scroll");
    }

    #[test]
    fn test_set_expanded_diff_sets_diff() {
        let mut state = AppState::new(
            vec![make_entry("src/ui/mod.rs", 7, 3)],
            Duration::from_millis(600),
            "main".to_string(),
        );
        let diff = FileDiff::default();
        state.set_expanded_diff("src/ui/mod.rs".to_string(), diff);
        assert!(state.expanded_diff().is_some());
        assert_eq!(state.expanded_diff_path(), Some("src/ui/mod.rs"));
        assert_eq!(state.expanded_diff_stats(), Some((7, 3)));
        assert_eq!(state.diff_scroll(), 0);
    }

    #[test]
    fn test_switch_dialog_visibility_roundtrip() {
        use crate::git::worktree::WorktreeEntry;
        use crate::ui::switch_dialog::SwitchDialog;
        use std::path::{Path, PathBuf};

        let mut s = AppState::new(Vec::new(), Duration::from_millis(100), "main".to_string());
        assert!(!s.is_switch_dialog_visible());

        let entries = vec![WorktreeEntry {
            path: PathBuf::from("/a"),
            head: "0000000000000000000000000000000000000000".to_string(),
            branch: Some("main".to_string()),
            bare: false,
            detached: false,
            locked: None,
            prunable: None,
        }];
        let dialog = SwitchDialog::new(entries, Path::new("/a"), Path::new("/"));
        s.show_switch_dialog(dialog);
        assert!(s.is_switch_dialog_visible());
        assert!(s.switch_dialog().is_some());
        assert!(s.switch_dialog_mut().is_some());

        s.hide_switch_dialog();
        assert!(!s.is_switch_dialog_visible());
        assert!(s.switch_dialog().is_none());
    }

    #[test]
    fn test_reset_for_switch_clears_diff_overlay() {
        let mut state = make_state();
        state.show_diff_overlay();
        state.set_expanded_diff("src/ui/mod.rs".to_string(), crate::git::FileDiff::default());
        state.scroll_diff_down();
        assert!(state.is_diff_overlay_visible());
        assert!(state.expanded_diff().is_some());
        assert_eq!(state.diff_scroll(), 1);

        let before_token = state.pending_diff_token();
        state.reset_for_switch(Vec::new(), String::new(), String::new(), String::new());

        assert!(!state.is_diff_overlay_visible());
        assert!(state.expanded_diff().is_none());
        assert_eq!(state.diff_scroll(), 0);
        assert_ne!(
            state.pending_diff_token(),
            before_token,
            "token should advance"
        );
    }

    #[test]
    fn test_select_first_and_last() {
        let files = vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 2, 1),
            make_entry("c.rs", 0, 3),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.select_last();
        assert_eq!(state.selected_index(), 2);
        state.select_first();
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_select_page_clamps_both_ends() {
        let files = vec![
            make_entry("a.rs", 1, 0),
            make_entry("b.rs", 2, 1),
            make_entry("c.rs", 0, 3),
        ];
        let mut state = AppState::new(files, Duration::from_millis(600), "main".to_string());
        state.select_page(10); // past the end -> clamp to last
        assert_eq!(state.selected_index(), 2);
        state.select_page(-10); // past the start -> clamp to 0
        assert_eq!(state.selected_index(), 0);
        state.select_page(1);
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn test_select_page_empty_list_is_noop() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        state.select_page(5);
        assert_eq!(state.selected_index(), 0);
        state.select_last();
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_list_viewport_height_roundtrip() {
        let mut state = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        assert_eq!(state.list_viewport_height(), 0);
        state.set_list_viewport_height(20);
        assert_eq!(state.list_viewport_height(), 20);
    }

    fn diff_with_lines(n: usize) -> crate::git::FileDiff {
        use crate::git::{DiffHunk, DiffLine, DiffLineKind, FileDiff};
        let lines = (0..n)
            .map(|i| DiffLine {
                kind: DiffLineKind::Context,
                content: format!("line {i}"),
            })
            .collect();
        FileDiff {
            hunks: vec![DiffHunk {
                header: "@@ -1,1 +1,1 @@".to_string(),
                lines,
            }],
        }
    }

    #[test]
    fn test_diff_total_lines_counts_header_plus_lines() {
        let mut state = make_state();
        state.set_expanded_diff("a.rs".to_string(), diff_with_lines(9));
        // 1 hunk header + 9 content lines.
        assert_eq!(state.diff_total_lines(), 10);
    }

    #[test]
    fn test_scroll_diff_to_bottom_clamps_to_max() {
        let mut state = make_state();
        state.set_expanded_diff("a.rs".to_string(), diff_with_lines(19)); // 20 total
        state.set_diff_viewport_height(5);
        state.scroll_diff_to_bottom();
        // max scroll = 20 - 5 = 15
        assert_eq!(state.diff_scroll(), 15);
        state.scroll_diff_to_top();
        assert_eq!(state.diff_scroll(), 0);
    }

    #[test]
    fn test_scroll_diff_page_clamps() {
        let mut state = make_state();
        state.set_expanded_diff("a.rs".to_string(), diff_with_lines(19)); // 20 total
        state.set_diff_viewport_height(5); // max scroll = 15
        state.scroll_diff_page(100);
        assert_eq!(state.diff_scroll(), 15);
        state.scroll_diff_page(-3);
        assert_eq!(state.diff_scroll(), 12);
        state.scroll_diff_page(-100);
        assert_eq!(state.diff_scroll(), 0);
    }

    #[test]
    fn test_scroll_diff_to_bottom_when_content_fits_stays_zero() {
        let mut state = make_state();
        state.set_expanded_diff("a.rs".to_string(), diff_with_lines(2)); // 3 total
        state.set_diff_viewport_height(50); // viewport bigger than content
        state.scroll_diff_to_bottom();
        assert_eq!(state.diff_scroll(), 0);
    }
}
