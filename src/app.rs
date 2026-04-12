use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};

use crate::config::AppConfig;
use crate::git::GitRepo;
use crate::state::{AppState, Tab};
use crate::ui::Terminal;
use crate::watcher::{FsWatcher, FsWatcherEvent};

/// Events that drive the application
pub enum AppEvent {
    /// A filesystem change was detected (debounced)
    FsChange,
    /// A terminal event (key press, mouse, resize)
    Term(TermEvent),
    /// Periodic tick for UI refresh
    Tick,
}

/// Minimum time between automatic worktree switches.
const SWITCH_COOLDOWN: Duration = Duration::from_secs(3);

pub struct App {
    state: AppState,
    git: GitRepo,
    _watcher: FsWatcher,
    fs_rx: Receiver<FsWatcherEvent>,
    config: AppConfig,
    theme: crate::theme::Theme,
    tick_rate: Duration,
    /// The root repo path (for resolving worktrees)
    repo_path: PathBuf,
    /// The path currently being watched
    watch_path: PathBuf,
    /// Worktree monitor (None if auto-follow is disabled)
    worktree_monitor: Option<crate::watcher::worktree::WorktreeMonitor>,
    /// Receiver for worktree events
    wt_rx: Option<Receiver<crate::watcher::worktree::WorktreeEvent>>,
    /// Last time we switched worktrees (for cooldown)
    last_switch: Option<Instant>,
    /// Receiver for GitHub PR events
    gh_rx: Option<Receiver<crate::github::GitHubEvent>>,
}

impl App {
    pub fn new(
        watch_path: PathBuf,
        repo_path: PathBuf,
        config: AppConfig,
        debounce_ms: u64,
        auto_follow: bool,
        theme_override: Option<String>,
    ) -> Result<Self> {
        let git = GitRepo::new(&watch_path).context("Failed to open git repository")?;

        let files = git.status()?;
        let branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());

        let flash_duration = Duration::from_millis(config.display.flash_duration_ms);
        let mut state = AppState::new(files, flash_duration, branch);

        state.set_repo_name(git.repo_name());
        state.set_worktree_name(git.worktree_name());

        if let Ok((sha, msg)) = git.head_info() {
            state.set_head_info(sha, msg);
        }
        state.set_stash_count(git.stash_count().unwrap_or(0));
        state.set_ahead_behind(git.ahead_behind().unwrap_or(None));
        state.set_repo_state(git.repo_state());

        let user_themes_dir = crate::theme::default_user_themes_dir();
        let theme_name_or_path = theme_override.as_deref().unwrap_or(&config.theme);
        let theme = crate::theme::load_theme(theme_name_or_path, user_themes_dir.as_deref());

        let debounce = Duration::from_millis(debounce_ms);
        let (fs_rx, watcher) = FsWatcher::new(&watch_path, debounce)?;

        let (wt_rx, worktree_monitor) = if auto_follow {
            let (rx, mut monitor) =
                crate::watcher::worktree::WorktreeMonitor::new(&repo_path, debounce)?;
            monitor.set_current_target(None);
            (Some(rx), Some(monitor))
        } else {
            (None, None)
        };

        let gh_rx = if config.pr.enabled {
            if let Some(token) = crate::github::resolve_auth_token() {
                let branch = state.branch().to_string();
                Some(crate::github::start_polling(&watch_path, &branch, &token))
            } else {
                tracing::warn!("PR widget enabled but no GitHub auth token found");
                None
            }
        } else {
            None
        };

        Ok(Self {
            state,
            git,
            _watcher: watcher,
            fs_rx,
            config,
            theme,
            tick_rate: Duration::from_millis(250),
            repo_path,
            watch_path,
            worktree_monitor,
            wt_rx,
            last_switch: None,
            gh_rx,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = Terminal::new()?;
        terminal.setup()?;

        let result = self.event_loop(&mut terminal);

        terminal.teardown()?;
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal) -> Result<()> {
        let mut last_tick = Instant::now();

        loop {
            // Render current state
            terminal.draw(&self.state, &self.config, &self.theme)?;

            // Calculate timeout until next tick
            let timeout = self
                .tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_millis(0));

            // Multiplex event sources
            // Check for terminal events with timeout
            if event::poll(timeout.min(Duration::from_millis(50)))? {
                let term_event = event::read()?;
                if self.handle_terminal_event(term_event)? {
                    return Ok(());
                }
            }

            // Check for filesystem events (non-blocking).
            //
            // Both `FsChange` and `HeadChange` currently trigger a git
            // status recompute. Keeping the variants distinct at the
            // watcher level lets future work (e.g. commit list refresh)
            // differentiate without re-architecting the watcher.
            while let Ok(event) = self.fs_rx.try_recv() {
                match event {
                    FsWatcherEvent::FsChange | FsWatcherEvent::HeadChange => {
                        self.handle_fs_change()?;
                    }
                }
            }

            // Handle worktree events — drain into a vec first to release the borrow on self.wt_rx
            let wt_events: Vec<_> = self
                .wt_rx
                .as_ref()
                .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect())
                .unwrap_or_default();
            for wt_event in wt_events {
                self.handle_worktree_event(wt_event)?;
            }

            // Handle GitHub PR events
            if let Some(ref gh_rx) = self.gh_rx {
                while let Ok(event) = gh_rx.try_recv() {
                    match event {
                        crate::github::GitHubEvent::PrUpdate(info) => {
                            tracing::debug!(pr = info.number, "PR data updated");
                            self.state.set_pr_info(info);
                        }
                        crate::github::GitHubEvent::NoPr => {
                            tracing::debug!("No open PR found for current branch");
                            // Only clear if we don't have a sticky error
                            if self.state.pr_state().error.is_none() {
                                self.state.clear_pr();
                            }
                        }
                        crate::github::GitHubEvent::Error(err) => {
                            tracing::warn!(error = %err, "GitHub API error");
                            self.state.set_pr_error(err);
                        }
                    }
                }
            }

            // Tick
            if last_tick.elapsed() >= self.tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    /// Handle terminal input events. Returns true if the app should quit.
    fn handle_terminal_event(&mut self, event: TermEvent) -> Result<bool> {
        match event {
            TermEvent::Key(key) => {
                // Overlay mode: intercept keys before normal handling.
                // Only the Changes tab has an overlay; PR is read-only.
                if self.state.active_tab() == Tab::Changes && self.state.is_overlay_visible() {
                    match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),
                        (_, KeyCode::Esc)
                        | (_, KeyCode::Char('q'))
                        | (_, KeyCode::Char('h'))
                        | (_, KeyCode::Left) => self.state.hide_overlay(),
                        (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                            self.state.scroll_diff_down()
                        }
                        (_, KeyCode::Char('k')) | (_, KeyCode::Up) => self.state.scroll_diff_up(),
                        _ => {}
                    }
                    return Ok(false);
                }

                match (key.modifiers, key.code) {
                    // Tab switching — intercept first, before any per-tab routing.
                    (_, KeyCode::Tab) => {
                        self.state.next_tab();
                        return Ok(false);
                    }
                    (_, KeyCode::BackTab) => {
                        self.state.prev_tab();
                        return Ok(false);
                    }
                    (_, KeyCode::Char('1')) => {
                        self.state.set_tab(Tab::Changes);
                        return Ok(false);
                    }
                    (_, KeyCode::Char('2')) => {
                        self.state.set_tab(Tab::Pr);
                        return Ok(false);
                    }

                    // Quit
                    (_, KeyCode::Char('q')) => return Ok(true),
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),

                    // Navigation — per-tab routing (PR tab is read-only).
                    (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                        if self.state.active_tab() == Tab::Changes {
                            self.state.select_next();
                        }
                    }
                    (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                        if self.state.active_tab() == Tab::Changes {
                            self.state.select_previous();
                        }
                    }

                    // Expand / open diff
                    (_, KeyCode::Enter) | (_, KeyCode::Char('l')) | (_, KeyCode::Right) => {
                        if self.state.active_tab() == Tab::Changes {
                            self.handle_expand()?;
                        }
                    }
                    (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
                        if self.state.active_tab() == Tab::Changes {
                            self.state.collapse_selected();
                        }
                    }

                    // Refresh manually
                    (_, KeyCode::Char('r')) => {
                        self.handle_fs_change()?;
                    }

                    // Open external difftool (stub)
                    (_, KeyCode::Char('d')) => {
                        // TODO: open external difftool
                    }

                    _ => {}
                }
            }
            TermEvent::FocusGained => {
                self.state.set_focused(true);
            }
            TermEvent::FocusLost => {
                self.state.set_focused(false);
            }
            TermEvent::Resize(_, _) => {
                // ratatui handles resize automatically on next draw
            }
            _ => {}
        }
        Ok(false)
    }

    /// Recompute git status on filesystem change
    fn handle_fs_change(&mut self) -> Result<()> {
        tracing::debug!("Filesystem change detected, recomputing status");

        match self.git.status() {
            Ok(files) => {
                tracing::debug!(file_count = files.len(), "Git status returned");
                self.state.update_files(files);
            }
            Err(e) if e.is_env_change() => {
                tracing::debug!(error = %e, "git env changed during status, holding state");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        if let Ok(branch) = self.git.branch_name() {
            self.state.set_branch(branch);
        }
        if let Ok((sha, msg)) = self.git.head_info() {
            self.state.set_head_info(sha, msg);
        }
        if let Ok(count) = self.git.stash_count() {
            self.state.set_stash_count(count);
        }
        if let Ok(ab) = self.git.ahead_behind() {
            self.state.set_ahead_behind(ab);
        }
        self.state.set_repo_state(self.git.repo_state());

        Ok(())
    }

    /// Expand or show overlay diff for the currently selected file, depending on config
    fn handle_expand(&mut self) -> Result<()> {
        if let Some(path) = self.state.selected_path() {
            if self.config.keys.enter == "inline" {
                // Inline toggle behavior
                if self.state.is_expanded(&path) {
                    self.state.collapse_selected();
                } else {
                    match self.git.diff_file(&path) {
                        Ok(diff) => self.state.expand_selected(diff),
                        Err(e) if e.is_env_change() => {
                            tracing::debug!(error = %e, "git env changed during diff, skipping expand");
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            } else {
                // Overlay behavior (default)
                match self.git.diff_file(&path) {
                    Ok(diff) => {
                        self.state.expand_selected(diff);
                        self.state.show_overlay();
                    }
                    Err(e) if e.is_env_change() => {
                        tracing::debug!(error = %e, "git env changed during diff, skipping expand");
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }

    fn handle_worktree_event(
        &mut self,
        event: crate::watcher::worktree::WorktreeEvent,
    ) -> Result<()> {
        use crate::watcher::worktree::WorktreeEvent;

        match event {
            WorktreeEvent::Added(info) => {
                tracing::info!(worktree = %info.name, "New worktree detected");
                // Don't auto-switch — wait for activity to indicate user is working there
            }
            WorktreeEvent::Removed(name) => {
                tracing::info!(worktree = %name, "Worktree removed");
                let is_current = self
                    .worktree_monitor
                    .as_ref()
                    .and_then(|m| m.current_target())
                    .map(|t| t == name)
                    .unwrap_or(false);
                if is_current {
                    if let Some(ref monitor) = self.worktree_monitor {
                        if let Some(fallback) = monitor.most_recent_other() {
                            let info = fallback.clone();
                            self.switch_to_worktree(info)?;
                        } else {
                            self.switch_to_path(self.repo_path.clone())?;
                        }
                    }
                }
            }
            WorktreeEvent::Activity(name) => {
                tracing::debug!(worktree = %name, "Activity in worktree");
                if let Some(ref mut monitor) = self.worktree_monitor {
                    monitor.record_activity(&name);

                    // Check cooldown before switching
                    let cooldown_elapsed = self
                        .last_switch
                        .map(|t| t.elapsed() >= SWITCH_COOLDOWN)
                        .unwrap_or(true);

                    if cooldown_elapsed {
                        if let Some(most_recent) = monitor.most_recent_other() {
                            if most_recent.name == name {
                                let info = most_recent.clone();
                                self.switch_to_worktree(info)?;
                            }
                        }
                    }
                }
            }
            WorktreeEvent::StructureChanged => {
                if let Some(ref mut monitor) = self.worktree_monitor {
                    monitor.scan_and_reconcile();
                }
            }
        }

        Ok(())
    }

    fn switch_to_worktree(&mut self, info: crate::watcher::worktree::WorktreeInfo) -> Result<()> {
        tracing::info!(worktree = %info.name, path = ?info.path, "Switching to worktree");
        let name = info.name.clone();
        self.switch_to_path(info.path)?;
        self.state
            .set_flash_message(format!("Switched to worktree: {name}"));
        if let Some(ref mut monitor) = self.worktree_monitor {
            monitor.set_current_target(Some(name));
        }
        self.last_switch = Some(Instant::now());
        Ok(())
    }

    fn switch_to_path(&mut self, path: PathBuf) -> Result<()> {
        if path == self.watch_path {
            return Ok(());
        }
        let git = GitRepo::new(&path).context("Failed to open git repository at new path")?;

        let debounce = Duration::from_millis(self.config.debounce_ms);
        let (fs_rx, watcher) = FsWatcher::new(&path, debounce)?;

        let files = git.status()?;
        let branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());
        let repo_name = git.repo_name();
        let worktree_name = git.worktree_name();

        self.state
            .reset_for_switch(files, branch, repo_name, worktree_name);

        if let Ok((sha, msg)) = git.head_info() {
            self.state.set_head_info(sha, msg);
        }
        self.state.set_stash_count(git.stash_count().unwrap_or(0));
        self.state
            .set_ahead_behind(git.ahead_behind().unwrap_or(None));
        self.state.set_repo_state(git.repo_state());

        self.git = git;
        self._watcher = watcher;
        self.fs_rx = fs_rx;
        self.watch_path = path;

        // Restart the GitHub PR poller for the new branch. Without this,
        // the old poller thread keeps running against the old branch and
        // re-populates `pr_state` after `reset_for_switch` clears it,
        // making the PR tab reappear for the wrong branch.
        //
        // Dropping the current `gh_rx` causes the old poller thread to
        // exit on its next send attempt (poller.rs:100 breaks the loop
        // when `send()` returns `Err` because the receiver is dropped).
        self.gh_rx = if self.config.pr.enabled {
            if let Some(token) = crate::github::resolve_auth_token() {
                Some(crate::github::start_polling(
                    &self.watch_path,
                    self.state.branch(),
                    &token,
                ))
            } else {
                None
            }
        } else {
            None
        };

        Ok(())
    }
}
