use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};

use crate::config::AppConfig;
use crate::git::GitRepo;
use crate::state::AppState;
use crate::ui::Terminal;
use crate::watcher::FsWatcher;

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
    fs_rx: Receiver<()>,
    config: AppConfig,
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
}

impl App {
    pub fn new(
        watch_path: PathBuf,
        repo_path: PathBuf,
        config: AppConfig,
        debounce_ms: u64,
        auto_follow: bool,
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

        Ok(Self {
            state,
            git,
            _watcher: watcher,
            fs_rx,
            config,
            tick_rate: Duration::from_millis(250),
            repo_path,
            watch_path,
            worktree_monitor,
            wt_rx,
            last_switch: None,
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
            let theme = crate::theme::get_theme(&self.config.theme);
            terminal.draw(&self.state, &self.config, theme)?;

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

            // Check for filesystem events (non-blocking)
            while self.fs_rx.try_recv().is_ok() {
                self.handle_fs_change()?;
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
                match (key.modifiers, key.code) {
                    // Quit
                    (_, KeyCode::Char('q')) => return Ok(true),
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),

                    // Navigation
                    (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                        self.state.select_next();
                    }
                    (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                        self.state.select_previous();
                    }

                    // Expand / collapse diff
                    (_, KeyCode::Enter) | (_, KeyCode::Char('l')) | (_, KeyCode::Right) => {
                        self.toggle_expand()?;
                    }
                    (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
                        self.state.collapse_selected();
                    }

                    // Refresh manually
                    (_, KeyCode::Char('r')) => {
                        self.handle_fs_change()?;
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
        let files = self.git.status()?;
        let branch = self
            .git
            .branch_name()
            .unwrap_or_else(|_| "HEAD".to_string());
        tracing::debug!(file_count = files.len(), "Git status returned");
        for f in &files {
            tracing::debug!(path = %f.path, ins = f.insertions, del = f.deletions, "  file");
        }
        self.state.set_branch(branch);
        self.state.update_files(files);

        if let Ok((sha, msg)) = self.git.head_info() {
            self.state.set_head_info(sha, msg);
        }
        self.state
            .set_stash_count(self.git.stash_count().unwrap_or(0));
        self.state
            .set_ahead_behind(self.git.ahead_behind().unwrap_or(None));
        self.state.set_repo_state(self.git.repo_state());

        Ok(())
    }

    /// Toggle expanded diff for the currently selected file
    fn toggle_expand(&mut self) -> Result<()> {
        if let Some(path) = self.state.selected_path() {
            if self.state.is_expanded(&path) {
                self.state.collapse_selected();
            } else {
                // Compute diff for this file
                let diff = self.git.diff_file(&path)?;
                self.state.expand_selected(diff);
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

        Ok(())
    }
}
