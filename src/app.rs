use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use crossterm::event::{
    self, DisableFocusChange, EnableFocusChange, Event as TermEvent, KeyCode, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use crate::config::AppConfig;
use crate::git::GitRepo;
use crate::state::AppState;
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
    /// The path of the main worktree — stable fallback target when
    /// the watched worktree is removed.
    main_worktree_path: PathBuf,
    /// Tracks whether we have already warned the user that the main worktree
    /// is missing. Prevents the tick-level existence check from spamming the
    /// flash message every 250 ms when both the watched path and the main
    /// worktree are gone.
    main_missing_warned: bool,
    /// Worktree monitor (None if auto-follow is disabled)
    worktree_monitor: Option<crate::watcher::worktree::WorktreeMonitor>,
    /// Receiver for worktree events
    wt_rx: Option<Receiver<crate::watcher::worktree::WorktreeEvent>>,
    /// Receiver for GitHub PR events
    gh_rx: Option<Receiver<crate::github::GitHubEvent>>,
    /// CLI override for base branch
    base_override: Option<String>,
}

/// The action the tick-level existence check or `Removed` handler should take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FallbackAction {
    /// Watched path still exists — do nothing.
    None,
    /// Watched path is gone but the main worktree is available — switch to it.
    SwitchToMain,
    /// Watched path is gone and the main worktree is also missing — hold state
    /// and surface a user-visible flash.
    MainMissing,
}

/// Decide whether to fall back to the main worktree based on filesystem state.
pub(crate) fn fallback_decision(
    watch_path: &std::path::Path,
    main_path: &std::path::Path,
) -> FallbackAction {
    if watch_path.exists() {
        return FallbackAction::None;
    }
    if watch_path == main_path || !main_path.exists() {
        return FallbackAction::MainMissing;
    }
    FallbackAction::SwitchToMain
}

/// Resolve the shell command used to edit a file.
///
/// Order: `config.edit_command` → `$EDITOR` (if set and non-empty) → `"vim"`.
fn resolve_editor(config: &crate::config::AppConfig) -> String {
    if let Some(cmd) = config.edit_command.as_ref() {
        if !cmd.is_empty() {
            return cmd.clone();
        }
    }
    if let Ok(cmd) = std::env::var("EDITOR") {
        if !cmd.is_empty() {
            return cmd;
        }
    }
    "vim".to_string()
}

/// Wrap a string in POSIX single-quotes, escaping any embedded single-quotes
/// via the `'\''` idiom. Safe against spaces, `$`, backticks, quotes, etc.
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Build the shell command string for opening a file diff via git's configured
/// pager. When `merge_base` is `Some`, produces a branch-scoped diff matching
/// the in-app branch view (merge-base vs working tree). Otherwise produces a
/// working-tree-vs-index diff. `quoted_path` MUST be pre-quoted via
/// `shell_single_quote`.
fn build_diff_command(merge_base: Option<&gix::ObjectId>, quoted_path: &str) -> String {
    match merge_base {
        Some(mb) => format!("git diff {} -- {}", mb.to_hex(), quoted_path),
        None => format!("git diff -- {}", quoted_path),
    }
}

/// Open a URL in the user's default browser (macOS `open`, Linux `xdg-open`,
/// Windows `cmd /C start`). Detached: returns immediately, git-rt keeps running.
/// Stdio is nulled so launcher noise doesn't corrupt the TUI.
fn open_url(url: &str) -> Result<()> {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        // `start` treats its first quoted argument as the window title, so we
        // pass an empty title before the URL.
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("failed to launch browser via {program}"))?;
    Ok(())
}

/// RAII guard around a foreground child process. Suspend() leaves raw mode and
/// the alt screen; Drop restores them (and clears the screen) so ratatui redraws
/// cleanly. Drop runs on panic, so the terminal is always restored.
struct TerminalGuard<'a> {
    terminal: &'a mut Terminal,
}

impl<'a> TerminalGuard<'a> {
    fn suspend(terminal: &'a mut Terminal) -> Result<Self> {
        disable_raw_mode().context("disable_raw_mode")?;
        execute!(std::io::stdout(), LeaveAlternateScreen, DisableFocusChange)
            .context("LeaveAlternateScreen + DisableFocusChange")?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard<'_> {
    fn drop(&mut self) {
        let _ = enable_raw_mode();
        let _ = execute!(std::io::stdout(), EnterAlternateScreen, EnableFocusChange);
        let _ = self.terminal.clear();
    }
}

impl App {
    pub fn new(
        watch_path: PathBuf,
        repo_path: PathBuf,
        config: AppConfig,
        debounce_ms: u64,
        auto_follow: bool,
        theme_override: Option<String>,
        base_override: Option<String>,
    ) -> Result<Self> {
        let git = GitRepo::new(&watch_path).context("Failed to open git repository")?;

        let branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());

        let (merge_base, resolved_base, files) = Self::compute_branch_files(
            &git,
            base_override.as_deref(),
            config.base_branch.as_deref(),
        )?;

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
        state.set_merge_base(merge_base, resolved_base);

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

        let main_worktree_path = crate::git::main_worktree_path(&repo_path);

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
            main_worktree_path,
            main_missing_warned: false,
            worktree_monitor,
            wt_rx,
            gh_rx,
            base_override,
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
                if self.handle_terminal_event(term_event, terminal)? {
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

                // If the watched directory disappeared without a Removed
                // event (e.g. `rm -rf`), fall back to the main worktree.
                if matches!(
                    fallback_decision(&self.watch_path, &self.main_worktree_path),
                    FallbackAction::SwitchToMain | FallbackAction::MainMissing,
                ) {
                    self.fallback_to_main()?;
                }
            }
        }
    }

    /// Handle terminal input events. Returns true if the app should quit.
    fn handle_terminal_event(&mut self, event: TermEvent, terminal: &mut Terminal) -> Result<bool> {
        match event {
            TermEvent::Key(key) => {
                // Help overlay mode: intercept keys before anything else.
                if self.state.is_help_visible() {
                    match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),
                        (_, KeyCode::Esc)
                        | (_, KeyCode::Char('q'))
                        | (_, KeyCode::Char('?'))
                        | (_, KeyCode::Char(' ')) => self.state.hide_help(),
                        _ => {}
                    }
                    return Ok(false);
                }

                // Diff overlay mode: intercept keys before normal handling.
                if self.state.is_overlay_visible() {
                    match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),
                        (_, KeyCode::Esc)
                        | (_, KeyCode::Char('q'))
                        | (_, KeyCode::Char('h'))
                        | (_, KeyCode::Char(' '))
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

                    // Expand / open diff (Enter, l, Right, Space)
                    (_, KeyCode::Enter)
                    | (_, KeyCode::Char('l'))
                    | (_, KeyCode::Right)
                    | (_, KeyCode::Char(' ')) => {
                        self.handle_expand()?;
                    }
                    (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
                        self.state.collapse_selected();
                    }

                    // Refresh manually
                    (_, KeyCode::Char('r')) => {
                        self.handle_fs_change()?;
                    }

                    // Edit selected file
                    (_, KeyCode::Char('e')) => {
                        self.edit_selected_file(terminal)?;
                    }

                    // Open detected PR in browser
                    (_, KeyCode::Char('p')) => {
                        self.open_pr()?;
                    }

                    // Help popup
                    (_, KeyCode::Char('?')) => {
                        self.state.show_help();
                    }

                    // Open selected file in the configured git pager
                    (_, KeyCode::Char('d')) => {
                        self.open_pager_diff(terminal)?;
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

    /// Resolve merge base and compute file list. Falls back to working-tree
    /// status when no merge base is available or on error.
    fn compute_branch_files(
        git: &GitRepo,
        base_override: Option<&str>,
        config_base: Option<&str>,
    ) -> Result<(Option<gix::ObjectId>, String, Vec<crate::git::FileEntry>)> {
        let base_ref = base_override.or(config_base);
        let resolved_base = git.resolve_base_branch(base_ref);

        let (merge_base, files) = match &resolved_base {
            Some(base_name) => match git.merge_base(base_name) {
                Ok(Some(mb)) => match git.branch_status(mb) {
                    Ok(f) => (Some(mb), f),
                    Err(e) if e.is_env_change() => {
                        tracing::debug!(error = %e, "branch_status failed, falling back");
                        (None, git.status()?)
                    }
                    Err(e) => return Err(e.into()),
                },
                Ok(None) => (None, git.status()?),
                Err(e) if e.is_env_change() => {
                    tracing::debug!(error = %e, "merge_base failed, falling back");
                    (None, git.status()?)
                }
                Err(e) => return Err(e.into()),
            },
            None => (None, git.status()?),
        };

        Ok((merge_base, resolved_base.unwrap_or_default(), files))
    }

    /// Recompute git status on filesystem change
    fn handle_fs_change(&mut self) -> Result<()> {
        tracing::debug!("Filesystem change detected, recomputing status");

        let base_ref = self
            .base_override
            .as_deref()
            .or(self.config.base_branch.as_deref());
        let resolved_base = self.git.resolve_base_branch(base_ref);
        let (merge_base, files_result) = match &resolved_base {
            Some(base_name) => match self.git.merge_base(base_name) {
                Ok(Some(mb)) => (Some(mb), self.git.branch_status(mb)),
                Ok(None) => (None, self.git.status()),
                Err(e) if e.is_env_change() => {
                    tracing::debug!(error = %e, "merge_base env change, holding state");
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            },
            None => (None, self.git.status()),
        };

        match files_result {
            Ok(files) => {
                tracing::debug!(file_count = files.len(), "Status returned");
                self.state.update_files(files);
                self.state
                    .set_merge_base(merge_base, resolved_base.unwrap_or_default());
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

    /// Open the currently selected file in an editor. No-op when no file is selected.
    fn edit_selected_file(&mut self, terminal: &mut Terminal) -> Result<()> {
        let Some(path) = self.state.selected_path() else {
            tracing::debug!("no file selected; skipping edit");
            return Ok(());
        };
        let editor = resolve_editor(&self.config);
        let quoted = shell_single_quote(&path);
        let cmd = format!("{editor} {quoted}");
        tracing::info!(%cmd, cwd = %self.watch_path.display(), "Launching editor");

        let _guard = TerminalGuard::suspend(terminal)?;
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(&self.watch_path)
            .status()
            .with_context(|| format!("failed to launch editor: {cmd}"))?;

        if !status.success() {
            tracing::info!(?status, "Editor exited non-zero");
        }
        Ok(())
    }

    /// Open the currently selected file in the configured git pager via
    /// `git diff`. Uses the app's merge base when set (matching the branch-
    /// scoped in-app view); otherwise runs a working-tree-vs-index diff.
    /// No-op when no file is selected.
    fn open_pager_diff(&mut self, terminal: &mut Terminal) -> Result<()> {
        let Some(path) = self.state.selected_path() else {
            tracing::debug!("no file selected; skipping open_pager_diff");
            return Ok(());
        };
        let quoted = shell_single_quote(&path);
        let cmd = build_diff_command(self.state.merge_base().as_ref(), &quoted);
        tracing::info!(%cmd, cwd = %self.watch_path.display(), "Launching git diff");

        let _guard = TerminalGuard::suspend(terminal)?;
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(&self.watch_path)
            .status()
            .with_context(|| format!("failed to launch git diff: {cmd}"))?;

        if !status.success() {
            tracing::info!(?status, "git diff exited non-zero");
        }
        Ok(())
    }

    /// Open the detected PR for the current branch in the default browser.
    /// No-op when no PR is detected.
    fn open_pr(&mut self) -> Result<()> {
        let Some(info) = self.state.pr_state().info.as_ref() else {
            tracing::debug!("no PR detected; skipping open_pr");
            return Ok(());
        };
        let url = info.url.clone();
        tracing::info!(%url, "Opening PR in browser");
        open_url(&url)
    }

    /// Expand or show overlay diff for the currently selected file, depending on config
    fn handle_expand(&mut self) -> Result<()> {
        if let Some(path) = self.state.selected_path() {
            let diff_result = if let Some(mb) = self.state.merge_base() {
                self.git.branch_diff_file(&path, mb)
            } else {
                self.git.diff_file(&path)
            };

            if self.config.keys.enter == "inline" {
                if self.state.is_expanded(&path) {
                    self.state.collapse_selected();
                } else {
                    match diff_result {
                        Ok(diff) => self.state.expand_selected(diff),
                        Err(e) if e.is_env_change() => {
                            tracing::debug!(error = %e, "git env changed during diff, skipping expand");
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            } else {
                match diff_result {
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
                tracing::info!(worktree = %info.name, "New worktree detected, switching");
                self.switch_to_worktree(info)?;
            }
            WorktreeEvent::Removed { name, path } => {
                tracing::info!(worktree = %name, path = ?path, "Worktree removed");
                let is_current_by_path = self.watch_path == path;
                let is_current_by_name = self
                    .worktree_monitor
                    .as_ref()
                    .and_then(|m| m.current_target())
                    .map(|t| t == name)
                    .unwrap_or(false);
                if is_current_by_path || is_current_by_name {
                    self.fallback_to_main()?;
                }
            }
            WorktreeEvent::BranchChanged { worktree, branch } => {
                tracing::debug!(worktree = %worktree, branch = %branch, "Branch change event");
                let is_current = self
                    .worktree_monitor
                    .as_ref()
                    .and_then(|m| m.current_target())
                    .map(|t| t == worktree)
                    .unwrap_or(false);

                if is_current {
                    // Current target — FsWatcher handles refresh; just record the branch
                    if let Some(ref mut monitor) = self.worktree_monitor {
                        monitor.record_branch(&worktree, &branch);
                    }
                } else if let Some(ref mut monitor) = self.worktree_monitor {
                    if monitor.is_branch_change(&worktree, &branch) {
                        monitor.record_branch(&worktree, &branch);
                        if let Some(info) = monitor.worktree_info(&worktree).cloned() {
                            self.switch_to_worktree(info)?;
                        }
                    } else {
                        // Same branch (e.g. ref update from commit) — record but don't switch
                        monitor.record_branch(&worktree, &branch);
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
        Ok(())
    }

    fn switch_to_path(&mut self, path: PathBuf) -> Result<()> {
        if path == self.watch_path {
            return Ok(());
        }
        let git = GitRepo::new(&path).context("Failed to open git repository at new path")?;

        let debounce = Duration::from_millis(self.config.debounce_ms);
        let (fs_rx, watcher) = FsWatcher::new(&path, debounce)?;

        let (merge_base, resolved_base, files) = Self::compute_branch_files(
            &git,
            self.base_override.as_deref(),
            self.config.base_branch.as_deref(),
        )?;

        let branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());
        let repo_name = git.repo_name();
        let worktree_name = git.worktree_name();

        self.state
            .reset_for_switch(files, branch, repo_name, worktree_name);
        self.state.set_merge_base(merge_base, resolved_base);

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

    /// Fall back to the main worktree when the watched worktree is gone.
    ///
    /// Uses `fallback_decision` to choose among switching, flashing a
    /// "main-missing" warning, or no-op. Idempotent — safe to call on
    /// every tick.
    fn fallback_to_main(&mut self) -> Result<()> {
        match fallback_decision(&self.watch_path, &self.main_worktree_path) {
            FallbackAction::None => {
                self.main_missing_warned = false;
                Ok(())
            }
            FallbackAction::SwitchToMain => {
                let main = self.main_worktree_path.clone();
                tracing::info!(
                    watch = ?self.watch_path,
                    main = ?main,
                    "Watched worktree removed — switching to main"
                );
                self.switch_to_path(main)?;
                self.state
                    .set_flash_message("Worktree removed — switched to main".to_string());
                if let Some(ref mut monitor) = self.worktree_monitor {
                    monitor.set_current_target(None);
                }
                self.main_missing_warned = false;
                Ok(())
            }
            FallbackAction::MainMissing => {
                if !self.main_missing_warned {
                    tracing::warn!(
                        watch = ?self.watch_path,
                        main = ?self.main_worktree_path,
                        "Watched worktree and main worktree both missing — holding state"
                    );
                    self.state
                        .set_flash_message("Main worktree is missing — holding state".to_string());
                    self.main_missing_warned = true;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_fallback_decision_watch_path_exists() {
        let tmp = tempdir().unwrap();
        let watch = tmp.path().join("wt");
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&watch).unwrap();
        std::fs::create_dir_all(&main).unwrap();

        assert_eq!(fallback_decision(&watch, &main), FallbackAction::None,);
    }

    #[test]
    fn test_fallback_decision_watch_gone_main_exists() {
        let tmp = tempdir().unwrap();
        let watch = tmp.path().join("wt"); // never created
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();

        assert_eq!(
            fallback_decision(&watch, &main),
            FallbackAction::SwitchToMain,
        );
    }

    #[test]
    fn test_fallback_decision_watch_equals_main() {
        let tmp = tempdir().unwrap();
        let shared = tmp.path().join("repo");
        // Intentionally do not create — emulate "main itself is missing"
        assert_eq!(
            fallback_decision(&shared, &shared),
            FallbackAction::MainMissing,
        );
    }

    #[test]
    fn test_fallback_decision_both_missing() {
        let tmp = tempdir().unwrap();
        let watch = tmp.path().join("wt");
        let main = tmp.path().join("main");
        assert_eq!(
            fallback_decision(&watch, &main),
            FallbackAction::MainMissing,
        );
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use crate::config::AppConfig;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Serializes tests that mutate the process-global `EDITOR` env var so
    /// they don't race when Cargo runs tests in parallel threads.
    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Guard that saves + restores a single env var around a block.
    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn config_value_wins() {
        let _lock = env_lock();
        let _g = EnvGuard::set("EDITOR", "emacs");
        let cfg = AppConfig {
            edit_command: Some("nvim -p".to_string()),
            ..AppConfig::default()
        };
        assert_eq!(resolve_editor(&cfg), "nvim -p");
    }

    #[test]
    fn falls_back_to_editor_env() {
        let _lock = env_lock();
        let _g = EnvGuard::set("EDITOR", "emacs");
        let cfg = AppConfig::default();
        assert_eq!(resolve_editor(&cfg), "emacs");
    }

    #[test]
    fn falls_back_to_vim_when_unset() {
        let _lock = env_lock();
        let _g = EnvGuard::remove("EDITOR");
        let cfg = AppConfig::default();
        assert_eq!(resolve_editor(&cfg), "vim");
    }

    #[test]
    fn empty_editor_env_falls_back_to_vim() {
        let _lock = env_lock();
        let _g = EnvGuard::set("EDITOR", "");
        let cfg = AppConfig::default();
        assert_eq!(resolve_editor(&cfg), "vim");
    }

    #[test]
    fn empty_edit_command_falls_through() {
        let _lock = env_lock();
        let _g = EnvGuard::set("EDITOR", "emacs");
        let cfg = AppConfig {
            edit_command: Some(String::new()),
            ..AppConfig::default()
        };
        assert_eq!(resolve_editor(&cfg), "emacs");
    }

    #[test]
    fn quote_plain() {
        assert_eq!(shell_single_quote("foo.rs"), "'foo.rs'");
    }

    #[test]
    fn quote_space() {
        assert_eq!(shell_single_quote("my file.rs"), "'my file.rs'");
    }

    #[test]
    fn quote_apostrophe() {
        assert_eq!(shell_single_quote("it's.rs"), "'it'\\''s.rs'");
    }

    #[test]
    fn quote_dollar_and_backtick() {
        assert_eq!(shell_single_quote("a$b`c.rs"), "'a$b`c.rs'");
    }

    #[test]
    fn quote_empty() {
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn build_diff_command_working_tree() {
        let cmd = build_diff_command(None, "'src/main.rs'");
        assert_eq!(cmd, "git diff -- 'src/main.rs'");
    }

    #[test]
    fn build_diff_command_branch_scoped() {
        let mb = gix::ObjectId::null(gix::hash::Kind::Sha1);
        let cmd = build_diff_command(Some(&mb), "'src/main.rs'");
        assert_eq!(
            cmd,
            "git diff 0000000000000000000000000000000000000000 -- 'src/main.rs'"
        );
    }

    #[test]
    fn build_diff_command_quoted_path_with_space() {
        let cmd = build_diff_command(None, "'my file.rs'");
        assert_eq!(cmd, "git diff -- 'my file.rs'");
    }
}
