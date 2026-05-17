use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use crossterm::event::{
    self, DisableFocusChange, EnableFocusChange, Event as TermEvent, KeyCode, KeyEvent,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use crate::config::AppConfig;
use crate::git::worker::{Request, Response, StatusBundle, Worker};
use crate::git::GitRepo;
use crate::state::AppState;
use crate::ui::Terminal;
use crate::watcher::{FsWatcher, FsWatcherEvent};

/// How often the event loop performs periodic work (flash-fade update,
/// watched-path-missing fallback). The loop also renders and drains channels
/// on every iteration regardless of this interval.
const TICK_RATE: Duration = Duration::from_millis(1000);

/// Maximum time `event::poll` waits for a terminal event before returning
/// control to the loop to drain the other (fs, worker, github) channels.
/// Does NOT affect keystroke latency: `event::poll` returns immediately
/// when a key arrives. This ceiling only bounds how stale non-keyboard
/// events can be when the user is idle.
const EVENT_POLL_MAX: Duration = Duration::from_millis(200);

/// Events that drive the application
pub enum AppEvent {
    /// A filesystem change was detected (debounced)
    FsChange,
    /// A terminal event (key press, mouse, resize)
    Term(TermEvent),
    /// Periodic tick for UI refresh
    Tick,
}

/// Payload sent from the background watcher-init thread once
/// `FsWatcher::new` finishes its recursive walk.
type WatcherReady = (FsWatcher, Receiver<FsWatcherEvent>);

/// Spawn a background thread that builds the `FsWatcher` for `watch_path`
/// and delivers the handle + receiver through the returned channel.
///
/// Used so `App::new` doesn't block on `FsWatcher::new`, which can take
/// multiple seconds on large monorepos while `notify-debouncer-full` seeds
/// its recursive cache. If construction fails the channel simply never
/// fires and live FS updates are disabled; a `tracing::error!` is emitted.
fn spawn_watcher_init(watch_path: PathBuf, debounce: Duration) -> Receiver<WatcherReady> {
    let (ready_tx, ready_rx) = bounded::<WatcherReady>(1);
    std::thread::spawn(move || {
        let t = Instant::now();
        match FsWatcher::new(&watch_path, debounce) {
            Ok((fs_rx, watcher)) => {
                tracing::debug!(
                    elapsed_ms = t.elapsed().as_millis() as u64,
                    "background: FsWatcher::new"
                );
                let _ = ready_tx.send((watcher, fs_rx));
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "background: FsWatcher::new failed; live updates disabled"
                );
            }
        }
    });
    ready_rx
}

pub struct App {
    state: AppState,
    /// `None` until the background watcher-init thread finishes; live FS
    /// updates are disabled during that window.
    _watcher: Option<FsWatcher>,
    /// `None` until the background watcher-init thread finishes.
    fs_rx: Option<Receiver<FsWatcherEvent>>,
    /// Receives the watcher handle + receiver once the background init
    /// completes. Cleared after installation.
    watcher_pending_rx: Option<Receiver<WatcherReady>>,
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
    /// Receiver for GitHub PR events
    gh_rx: Option<Receiver<crate::github::GitHubEvent>>,
    /// Last branch name observed in a recompute bundle. `None` until the
    /// first bundle arrives. Used by `apply_status` to detect mid-session
    /// branch renames so the PR poller can be restarted and a flash message
    /// shown.
    last_seen_branch: Option<String>,
    /// CLI override for base branch
    base_override: Option<String>,
    /// Sender for worker requests. Bounded; drops on overflow are safe
    /// since FS-driven recomputes are idempotent.
    worker_tx: Sender<Request>,
    /// Receiver for worker responses. Drained by the event loop in Task 5.
    worker_rx: Receiver<Response>,
    /// Join handle for the worker thread; stored so we can join on shutdown.
    worker_handle: Option<std::thread::JoinHandle<()>>,
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

/// Open a URL in the user's default browser (macOS `open`, Linux `xdg-open`,
/// Windows `cmd /C start`). Detached: returns immediately, perch keeps running.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainAction {
    Quit,
    MoveDown,
    MoveUp,
    Primary,
    Refresh,
    Edit,
    OpenPr,
    Help,
    CycleMode,
    OpenSwitcher,
    None,
}

fn interpret_main_key(modifiers: KeyModifiers, code: KeyCode) -> MainAction {
    match (modifiers, code) {
        (_, KeyCode::Char('q')) => MainAction::Quit,
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => MainAction::Quit,
        (_, KeyCode::Char('j')) | (_, KeyCode::Down) => MainAction::MoveDown,
        (_, KeyCode::Char('k')) | (_, KeyCode::Up) => MainAction::MoveUp,
        (_, KeyCode::Enter)
        | (_, KeyCode::Char('l'))
        | (_, KeyCode::Right)
        | (_, KeyCode::Char(' '))
        | (_, KeyCode::Char('d')) => MainAction::Primary,
        (_, KeyCode::Char('m')) => MainAction::CycleMode,
        (_, KeyCode::Char('r')) => MainAction::Refresh,
        (_, KeyCode::Char('e')) => MainAction::Edit,
        (_, KeyCode::Char('p')) => MainAction::OpenPr,
        (_, KeyCode::Char('?')) => MainAction::Help,
        (_, KeyCode::Char('s')) => MainAction::OpenSwitcher,
        _ => MainAction::None,
    }
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
        theme_override: Option<String>,
        base_override: Option<String>,
    ) -> Result<Self> {
        // Open the repo just long enough to read the branch name (sub-millisecond
        // — reads `.git/HEAD`). Everything else — file list, head_info, ahead/behind,
        // stash count, repo_state, merge_base — is deferred to the worker so
        // App::new returns instantly even on huge repos.
        let t = Instant::now();
        let git = GitRepo::new(&watch_path).context("Failed to open git repository")?;
        let branch = git.branch_name().unwrap_or_else(|_| "HEAD".to_string());
        drop(git); // worker re-opens its own GitRepo
        tracing::debug!(
            elapsed_ms = t.elapsed().as_millis() as u64,
            "App::new: GitRepo open + branch_name"
        );

        let flash_duration = Duration::from_millis(config.display.flash_duration_ms);
        let mut state = AppState::new(Vec::new(), flash_duration, branch.clone());
        state.set_view_mode(config.display.default_view);
        state.set_computing(true);

        let t = Instant::now();
        let user_themes_dir = crate::theme::default_user_themes_dir();
        let theme_name_or_path = theme_override.as_deref().unwrap_or(&config.theme);
        let theme = crate::theme::load_theme(theme_name_or_path, user_themes_dir.as_deref());
        tracing::debug!(
            elapsed_ms = t.elapsed().as_millis() as u64,
            "App::new: theme load"
        );

        // FsWatcher::new does a recursive walk of the working tree to seed
        // notify-debouncer-full's cache. On large monorepos that can take
        // multiple seconds; run it on a background thread so App::new (and
        // the first draw) returns immediately. Live FS updates are disabled
        // until the watcher is installed; a catch-up Recompute fires then.
        let t = Instant::now();
        let debounce = Duration::from_millis(debounce_ms);
        let watcher_pending_rx = spawn_watcher_init(watch_path.clone(), debounce);
        tracing::debug!(
            elapsed_ms = t.elapsed().as_millis() as u64,
            "App::new: spawn FsWatcher thread"
        );

        let t = Instant::now();
        let gh_rx = if config.pr.enabled {
            if let Some(token) = crate::github::resolve_auth_token() {
                Some(crate::github::start_polling(&watch_path, &branch, &token))
            } else {
                tracing::warn!("PR widget enabled but no GitHub auth token found");
                None
            }
        } else {
            None
        };
        tracing::debug!(
            elapsed_ms = t.elapsed().as_millis() as u64,
            "App::new: github polling setup"
        );

        let t = Instant::now();
        let main_worktree_path = crate::git::main_worktree_path(&repo_path);
        tracing::debug!(
            elapsed_ms = t.elapsed().as_millis() as u64,
            "App::new: main_worktree_path"
        );

        // Spawn the git worker thread and queue the initial Recompute. The
        // first Status response populates the file list and all branch metadata.
        let t = Instant::now();
        let (worker_tx, worker_req_rx) = bounded::<Request>(8);
        let (worker_resp_tx, worker_rx) = bounded::<Response>(8);
        let worker_handle = Worker::spawn(
            watch_path.clone(),
            base_override.clone(),
            config.base_branch.clone(),
            worker_req_rx,
            worker_resp_tx,
        );
        // Best-effort initial Recompute. If the channel is full or disconnected,
        // the worker is dead and the app would fail anyway — log and continue.
        crate::git::worker::warn_if_high(&worker_tx, "worker_req");
        if let Err(e) = worker_tx.try_send(Request::Recompute) {
            tracing::warn!(error = %e, "initial Recompute send failed");
            state.set_computing(false);
        }
        tracing::debug!(
            elapsed_ms = t.elapsed().as_millis() as u64,
            "App::new: Worker spawn + queue initial Recompute"
        );

        Ok(Self {
            state,
            _watcher: None,
            fs_rx: None,
            watcher_pending_rx: Some(watcher_pending_rx),
            config,
            theme,
            tick_rate: TICK_RATE,
            repo_path,
            watch_path,
            main_worktree_path,
            main_missing_warned: false,
            gh_rx,
            last_seen_branch: None,
            base_override,
            worker_tx,
            worker_rx,
            worker_handle: Some(worker_handle),
        })
    }

    #[tracing::instrument(name = "app.run", skip_all)]
    pub fn run(&mut self) -> Result<()> {
        let mut terminal = Terminal::new()?;
        terminal.setup()?;

        let result = self.event_loop(&mut terminal);

        // Shut the worker down cleanly so the thread exits before we drop App.
        let _ = self.worker_tx.send(Request::Shutdown);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }

        terminal.teardown()?;
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal) -> Result<()> {
        let mut last_tick = Instant::now();
        let loop_t0 = Instant::now();
        let mut first_draw_logged = false;

        loop {
            // Render current state
            terminal.draw(&mut self.state, &self.config, &self.theme)?;
            if !first_draw_logged {
                tracing::info!(
                    elapsed_ms = loop_t0.elapsed().as_millis() as u64,
                    "event_loop: first draw complete"
                );
                first_draw_logged = true;
            }

            // Calculate timeout until next tick
            let timeout = self
                .tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_millis(0));

            // Multiplex event sources
            // Check for terminal events with timeout
            if event::poll(timeout.min(EVENT_POLL_MAX))? {
                let term_event = event::read()?;
                if self.handle_terminal_event(term_event, terminal)? {
                    return Ok(());
                }
            }

            // Install the background-built FsWatcher if it's ready. Fires
            // a catch-up Recompute so any changes during setup are caught.
            if self._watcher.is_none() {
                let install = self
                    .watcher_pending_rx
                    .as_ref()
                    .and_then(|rx| rx.try_recv().ok());
                if let Some((watcher, fs_rx)) = install {
                    self._watcher = Some(watcher);
                    self.fs_rx = Some(fs_rx);
                    self.watcher_pending_rx = None;
                    tracing::info!("FsWatcher installed; issuing catch-up Recompute");
                    crate::git::worker::warn_if_high(&self.worker_tx, "worker_req");
                    if let Err(e) = self.worker_tx.try_send(Request::Recompute) {
                        tracing::warn!(error = %e, "catch-up Recompute send failed");
                    }
                }
            }

            // Check for filesystem events (non-blocking).
            //
            // Both `FsChange` and `HeadChange` currently trigger a git
            // status recompute. Keeping the variants distinct at the
            // watcher level lets future work (e.g. commit list refresh)
            // differentiate without re-architecting the watcher.
            //
            // Loop is structured so the immutable borrow of `self.fs_rx`
            // is dropped before the mutable `self.handle_fs_change` call.
            #[allow(clippy::while_let_loop)]
            loop {
                let event = match self.fs_rx.as_ref() {
                    Some(rx) => match rx.try_recv() {
                        Ok(e) => e,
                        Err(_) => break,
                    },
                    None => break,
                };
                match event {
                    FsWatcherEvent::FsChange | FsWatcherEvent::HeadChange => {
                        self.handle_fs_change()?;
                    }
                }
            }

            // Drain worker responses
            while let Ok(resp) = self.worker_rx.try_recv() {
                self.handle_worker_response(resp);
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
            TermEvent::Key(key) => self.handle_key_event(key, Some(terminal)),
            TermEvent::FocusGained => {
                self.state.set_focused(true);
                Ok(false)
            }
            TermEvent::FocusLost => {
                self.state.set_focused(false);
                Ok(false)
            }
            TermEvent::Resize(_, _) => {
                // ratatui handles resize automatically on next draw
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// Dispatch a key event. `terminal` is required for actions that suspend
    /// the UI (currently only `Edit`); when `None`, such actions are ignored.
    fn handle_key_event(&mut self, key: KeyEvent, terminal: Option<&mut Terminal>) -> Result<bool> {
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
        if self.state.is_diff_overlay_visible() {
            match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),
                (_, KeyCode::Esc)
                | (_, KeyCode::Char('q'))
                | (_, KeyCode::Char('h'))
                | (_, KeyCode::Left)
                | (_, KeyCode::Char(' '))
                | (_, KeyCode::Char('d')) => self.state.hide_diff_overlay(),
                (_, KeyCode::Char('j')) | (_, KeyCode::Down) => self.state.scroll_diff_down(),
                (_, KeyCode::Char('k')) | (_, KeyCode::Up) => self.state.scroll_diff_up(),
                _ => {}
            }
            return Ok(false);
        }

        // Switch dialog mode: intercept keys before normal handling.
        if self.state.is_switch_dialog_visible() {
            // Ctrl-C still quits.
            if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
                return Ok(true);
            }
            let outcome = self
                .state
                .switch_dialog_mut()
                .and_then(|d| d.handle_key(key));
            match outcome {
                None => {}
                Some(crate::ui::switch_dialog::DialogOutcome::Cancel) => {
                    self.state.hide_switch_dialog();
                }
                Some(crate::ui::switch_dialog::DialogOutcome::Reject(msg)) => {
                    self.state.set_flash_message(msg);
                }
                Some(crate::ui::switch_dialog::DialogOutcome::Switch(path)) => {
                    self.state.hide_switch_dialog();
                    if let Err(e) = self.switch_to_path(path) {
                        tracing::warn!(error = %e, "switch_to_path failed");
                        self.state.set_flash_message(format!("switch failed: {e}"));
                    }
                }
            }
            return Ok(false);
        }

        match interpret_main_key(key.modifiers, key.code) {
            MainAction::Quit => return Ok(true),
            MainAction::MoveDown => self.state.select_next(),
            MainAction::MoveUp => self.state.select_previous(),
            MainAction::Primary => self.handle_activate()?,
            MainAction::Refresh => self.handle_fs_change()?,
            MainAction::Edit => {
                if let Some(term) = terminal {
                    self.edit_selected_file(term)?;
                }
            }
            MainAction::OpenPr => self.open_pr()?,
            MainAction::Help => self.state.show_help(),
            MainAction::CycleMode => self.state.cycle_view_mode(),
            MainAction::OpenSwitcher => self.open_switch_dialog()?,
            MainAction::None => {}
        }
        Ok(false)
    }

    /// Send a Recompute request to the worker. Non-blocking. The response
    /// is applied later when the event loop drains `worker_rx` (Task 5).
    fn handle_fs_change(&mut self) -> Result<()> {
        tracing::debug!("Filesystem change detected; sending Recompute to worker");
        crate::git::worker::warn_if_high(&self.worker_tx, "worker_req");
        match self.worker_tx.try_send(Request::Recompute) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                tracing::debug!("Recompute dropped (channel full — already pending)");
                return Ok(());
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                tracing::warn!("git worker has exited; recompute dropped");
                self.state.set_computing(false);
                return Ok(());
            }
        }
        self.state.set_computing(true);
        Ok(())
    }

    /// React to a branch-name change in the watched worktree:
    /// 1. Restart the PR poller against the new branch.
    /// 2. Show a flash message with old → new.
    /// 3. Update `last_seen_branch`.
    ///
    /// Called by `apply_status` when a recompute bundle's branch name
    /// differs from the previously observed branch.
    fn handle_branch_change(&mut self, new_branch: &str) {
        let old = self.last_seen_branch.clone().unwrap_or_default();
        tracing::info!(old = %old, new = %new_branch, "branch renamed in watched worktree");

        // Clear stale PR data before restarting the poller so the UI doesn't
        // keep showing the old branch's PR while the new poller spins up.
        self.state.clear_pr();
        // Drop the old PR receiver. The sender thread will exit on its
        // next iteration when it tries to send on a dropped channel.
        self.gh_rx = None;
        if self.config.pr.enabled {
            if let Some(token) = crate::github::resolve_auth_token() {
                self.gh_rx = Some(crate::github::start_polling(
                    &self.watch_path,
                    new_branch,
                    &token,
                ));
            }
        }

        self.state
            .set_flash_message(format!("branch renamed: {old} → {new_branch}"));
        self.last_seen_branch = Some(new_branch.to_string());
    }

    /// Apply a worker `StatusBundle` to `AppState`.
    fn apply_status(&mut self, bundle: StatusBundle) {
        static FIRST_STATUS_LOGGED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if !FIRST_STATUS_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::info!(
                file_count = bundle.files.len(),
                "apply_status: first Status received"
            );
        }

        // Detect mid-session branch rename: only fire when we have a prior
        // observation that disagrees with the new bundle. The first bundle
        // sets `last_seen_branch` without firing the rename hook.
        match self.last_seen_branch.as_deref() {
            Some(prev) if prev != bundle.branch => {
                let new_branch = bundle.branch.clone();
                self.handle_branch_change(&new_branch);
            }
            None => {
                self.last_seen_branch = Some(bundle.branch.clone());
            }
            _ => {}
        }

        self.state.update_files(bundle.files);
        self.state
            .set_merge_base(bundle.merge_base, bundle.base_branch);
        self.state.set_branch(bundle.branch);
        if let Some((sha, msg)) = bundle.head {
            self.state.set_head_info(sha, msg);
        }
        self.state.set_stash_count(bundle.stash_count);
        self.state.set_ahead_behind(bundle.ahead_behind);
        self.state.set_repo_state(bundle.repo_state);
        self.state.set_repo_name(bundle.repo_name);
        self.state.set_worktree_name(bundle.worktree_name);
        self.state.set_computing(false);
    }

    fn handle_worker_response(&mut self, resp: Response) {
        match resp {
            Response::Status(bundle) => self.apply_status(*bundle),
            Response::Diff { path, token, diff } => {
                if token == self.state.pending_diff_token() {
                    self.state.set_expanded_diff(path, diff);
                    self.state.show_diff_overlay();
                } else {
                    tracing::debug!(
                        token,
                        current = self.state.pending_diff_token(),
                        ?path,
                        "Discarding stale diff response"
                    );
                }
            }
            Response::SwitchAck(_) => {
                // Handled inline by the SwitchRepo caller in Task 7.
            }
            Response::Error(msg) => {
                tracing::warn!(error = %msg, "Worker error");
                self.state.set_computing(false);
            }
        }
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

    /// Activate the selected row. Tree directories toggle open/closed;
    /// Expanded group headers toggle collapsed/expanded; file rows request
    /// their diff.
    fn handle_activate(&mut self) -> Result<()> {
        if self.state.toggle_selected_directory() {
            return Ok(());
        }

        if self.state.toggle_selected_group() {
            return Ok(());
        }

        self.handle_expand()
    }

    /// Send a Diff request to the worker for the currently selected file.
    /// The response is applied later when `worker_rx` is drained.
    fn handle_expand(&mut self) -> Result<()> {
        let Some(path) = self.state.selected_file_path() else {
            return Ok(());
        };

        let token = self.state.advance_pending_diff_token();

        match self.worker_tx.try_send(Request::Diff {
            path: path.clone(),
            token,
        }) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                tracing::debug!(token, "Diff request dropped (channel full)");
            }
            Err(e) => {
                return Err(anyhow::anyhow!("worker channel closed: {e}"));
            }
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

    /// Open the switch-worktree dialog. Closes the diff overlay if visible
    /// and flashes a message if `git worktree list` fails.
    fn open_switch_dialog(&mut self) -> Result<()> {
        if self.state.is_diff_overlay_visible() {
            self.state.hide_diff_overlay();
        }
        let entries = match crate::git::worktree::list(&self.repo_path) {
            Ok(es) => es,
            Err(e) => {
                tracing::warn!(error = %e, "git worktree list failed");
                self.state
                    .set_flash_message(format!("worktree list failed: {e}"));
                return Ok(());
            }
        };
        let current =
            std::fs::canonicalize(&self.watch_path).unwrap_or_else(|_| self.watch_path.clone());
        let primary = std::fs::canonicalize(&self.main_worktree_path)
            .unwrap_or_else(|_| self.main_worktree_path.clone());
        let dialog = crate::ui::switch_dialog::SwitchDialog::new(entries, &current, &primary);
        self.state.show_switch_dialog(dialog);
        Ok(())
    }

    fn switch_to_path(&mut self, path: PathBuf) -> Result<()> {
        if path == self.watch_path {
            return Ok(());
        }

        // Rebuild the FS watcher synchronously (cheap; doesn't open git).
        let debounce = Duration::from_millis(self.config.debounce_ms);
        let (fs_rx, watcher) = FsWatcher::new(&path, debounce)?;

        // Tell the worker to swap repos. Block briefly for the ack so we don't
        // race a new Recompute against the old repo path.
        self.worker_tx.send(Request::SwitchRepo(path.clone()))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                // 5s passed; bail. The error propagates to event_loop, which
                // returns; run() then triggers the Shutdown sequence so the
                // worker thread exits cleanly. App terminates rather than
                // continuing in a split worker/state condition.
                anyhow::bail!("worker did not ack SwitchRepo within 5s");
            }
            match self.worker_rx.recv_timeout(deadline - now) {
                Ok(Response::SwitchAck(true)) => break,
                Ok(Response::SwitchAck(false)) => {
                    anyhow::bail!("worker failed to switch repo to {:?}", path)
                }
                Ok(other) => {
                    // Drain non-ack responses — they're stale relative to the
                    // upcoming switch and the next Recompute will refresh.
                    tracing::debug!(?other, "discarding pre-switch worker response");
                    continue;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("worker thread disconnected during SwitchRepo");
                }
            }
        }

        // Worker is now on the new repo. Reset state and request a fresh load.
        // Reset the rename-detection baseline so the new worktree's first
        // bundle re-establishes it without firing a phantom rename.
        self.last_seen_branch = None;
        self.state
            .reset_for_switch(Vec::new(), String::new(), String::new(), String::new());
        self._watcher = Some(watcher);
        self.fs_rx = Some(fs_rx);
        self.watcher_pending_rx = None;
        self.watch_path = path;

        // Restart the GitHub PR poller for the new branch.
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

        // Trigger an initial Recompute on the new repo.
        crate::git::worker::warn_if_high(&self.worker_tx, "worker_req");
        let _ = self.worker_tx.try_send(Request::Recompute);
        self.state.set_computing(true);

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
mod watcher_init_tests {
    use super::*;
    use tempfile::tempdir;

    /// `spawn_watcher_init` must deliver a ready `FsWatcher` via the returned
    /// channel without blocking the caller. Returning immediately is the whole
    /// reason this helper exists — a regression that reintroduced a synchronous
    /// `FsWatcher::new` call would be invisible unless checked here.
    #[test]
    fn test_spawn_watcher_init_delivers_asynchronously() {
        let tmp = tempdir().unwrap();
        gix::init(tmp.path()).unwrap();

        let start = Instant::now();
        let rx = spawn_watcher_init(tmp.path().to_path_buf(), Duration::from_millis(100));
        // Returning the channel must not block on FsWatcher::new.
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "spawn_watcher_init blocked the caller for {:?}",
            start.elapsed()
        );

        // The watcher should arrive within a reasonable window on a tiny repo.
        let (_watcher, _fs_rx) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("watcher never delivered");
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
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::git::{ChangeGroup, FileDiff, FileEntry, FileStatus};
    use crossbeam_channel::Receiver;
    use crossterm::event::KeyEvent;

    fn make_entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 1,
            group: ChangeGroup::Changes,
        }
    }

    fn make_app(files: Vec<FileEntry>) -> App {
        make_app_with_requests(files).0
    }

    fn make_app_with_requests(files: Vec<FileEntry>) -> (App, Receiver<Request>) {
        let flash_duration = Duration::from_millis(AppConfig::default().display.flash_duration_ms);
        let (worker_tx, worker_req_rx) = bounded::<Request>(8);
        let (_worker_resp_tx, worker_rx) = bounded::<Response>(8);

        (
            App {
                state: AppState::new(files, flash_duration, "main".to_string()),
                _watcher: None,
                fs_rx: None,
                watcher_pending_rx: None,
                config: AppConfig::default(),
                theme: crate::theme::load_theme(crate::theme::DEFAULT_THEME_NAME, None),
                tick_rate: TICK_RATE,
                repo_path: PathBuf::new(),
                watch_path: PathBuf::new(),
                main_worktree_path: PathBuf::new(),
                main_missing_warned: false,
                gh_rx: None,
                last_seen_branch: None,
                base_override: None,
                worker_tx,
                worker_rx,
                worker_handle: None,
            },
            worker_req_rx,
        )
    }

    fn expect_diff_request(worker_req_rx: &Receiver<Request>) -> (String, u64) {
        match worker_req_rx.try_recv().expect("expected diff request") {
            Request::Diff { path, token } => (path, token),
            other => panic!("expected diff request, got {other:?}"),
        }
    }

    #[test]
    fn test_m_key_cycles_view_mode() {
        let mut app = make_app(vec![make_entry("src/ui/mod.rs")]);

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert_eq!(app.state.view_mode(), crate::state::ViewMode::Tree);
    }

    #[test]
    fn activate_on_group_header_toggles_collapse() {
        let files = vec![FileEntry {
            path: "a.rs".to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 0,
            group: ChangeGroup::Changes,
        }];
        let mut app = make_app(files);
        app.state.set_view_mode(crate::state::ViewMode::Expanded);
        // Row 0 is the "Changes" header; activate it.
        app.handle_activate().unwrap();
        assert_eq!(app.state.visible_rows().len(), 1); // file hidden
        assert_eq!(app.state.visible_rows()[0].header_collapsed(), Some(true));
    }

    #[test]
    fn test_interpret_main_key_maps_expected_actions() {
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Char('q')),
            MainAction::Quit
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::CONTROL, KeyCode::Char('c')),
            MainAction::Quit
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Down),
            MainAction::MoveDown
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Char('k')),
            MainAction::MoveUp
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Enter),
            MainAction::Primary
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Char('d')),
            MainAction::Primary
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Char('m')),
            MainAction::CycleMode
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Char('?')),
            MainAction::Help
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Esc),
            MainAction::None
        );
        assert_eq!(
            interpret_main_key(KeyModifiers::NONE, KeyCode::Char('s')),
            MainAction::OpenSwitcher
        );
    }

    #[test]
    fn test_enter_on_tree_directory_toggles_directory() {
        let mut app = make_app(vec![
            make_entry("src/ui/mod.rs"),
            make_entry("src/ui/header.rs"),
        ]);
        app.state.cycle_view_mode();
        app.state.select_previous();
        app.state.select_previous();

        assert_eq!(app.state.visible_rows().len(), 3);
        assert!(app.state.selected_path().is_none());

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert_eq!(app.state.visible_rows().len(), 1);
        assert!(!app.state.expanded_dirs().contains("src/ui"));
    }

    #[test]
    fn test_d_on_tree_directory_toggles_directory() {
        let mut app = make_app(vec![
            make_entry("src/ui/mod.rs"),
            make_entry("src/ui/header.rs"),
        ]);
        app.state.cycle_view_mode();
        app.state.select_previous();
        app.state.select_previous();

        assert_eq!(app.state.visible_rows().len(), 3);
        assert!(app.state.selected_path().is_none());

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert_eq!(app.state.visible_rows().len(), 1);
        assert!(!app.state.expanded_dirs().contains("src/ui"));
    }

    #[test]
    fn test_primary_key_on_tree_file_requests_diff() {
        let (mut app, worker_req_rx) = make_app_with_requests(vec![
            make_entry("src/ui/header.rs"),
            make_entry("src/ui/mod.rs"),
        ]);
        app.state.cycle_view_mode();

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        let (path, token) = expect_diff_request(&worker_req_rx);
        assert_eq!(path, "src/ui/header.rs");
        assert_eq!(token, app.state.pending_diff_token());
    }

    #[test]
    fn test_expand_ignores_tree_directory_selection() {
        let (mut app, worker_req_rx) = make_app_with_requests(vec![
            make_entry("src/ui/header.rs"),
            make_entry("src/ui/mod.rs"),
        ]);
        app.state.cycle_view_mode();
        app.state.select_previous();
        let token_before = app.state.pending_diff_token();

        app.handle_expand().unwrap();

        assert_eq!(app.state.pending_diff_token(), token_before);
        assert!(worker_req_rx.try_recv().is_err());
    }

    #[test]
    fn test_diff_response_stays_bound_to_requested_file_after_selection_moves() {
        let (mut app, worker_req_rx) = make_app_with_requests(vec![
            FileEntry {
                path: "src/ui/header.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 7,
                deletions: 3,
                group: ChangeGroup::Changes,
            },
            FileEntry {
                path: "src/ui/mod.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 2,
                deletions: 1,
                group: ChangeGroup::Changes,
            },
        ]);

        app.handle_expand().unwrap();
        let (requested_path, token) = expect_diff_request(&worker_req_rx);
        assert_eq!(requested_path, "src/ui/header.rs");

        app.state.select_next();
        assert_eq!(app.state.selected_path().as_deref(), Some("src/ui/mod.rs"));

        app.handle_worker_response(Response::Diff {
            path: requested_path.clone(),
            token,
            diff: FileDiff::default(),
        });

        assert!(app.state.is_diff_overlay_visible());
        assert_eq!(
            app.state.expanded_diff_path(),
            Some(requested_path.as_str())
        );
        assert_eq!(app.state.expanded_diff_stats(), Some((7, 3)));
    }

    #[test]
    fn test_help_overlay_still_intercepts_main_keys() {
        let mut app = make_app(vec![make_entry("src/ui/mod.rs")]);
        app.state.show_help();

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert!(app.state.is_help_visible());
        assert_eq!(app.state.view_mode(), crate::state::ViewMode::Flat);
    }

    fn make_dialog() -> crate::ui::switch_dialog::SwitchDialog {
        use crate::git::worktree::WorktreeEntry;
        use std::path::PathBuf;
        let entries = vec![
            WorktreeEntry {
                path: PathBuf::from("/a"),
                head: "0000000000000000000000000000000000000000".to_string(),
                branch: Some("main".to_string()),
                bare: false,
                detached: false,
                locked: None,
                prunable: None,
            },
            WorktreeEntry {
                path: PathBuf::from("/b"),
                head: "0000000000000000000000000000000000000000".to_string(),
                branch: Some("feat".to_string()),
                bare: false,
                detached: false,
                locked: None,
                prunable: None,
            },
        ];
        crate::ui::switch_dialog::SwitchDialog::new(
            entries,
            std::path::Path::new("/a"),
            std::path::Path::new("/"),
        )
    }

    #[test]
    fn test_s_while_help_visible_does_not_open_switch_dialog() {
        let mut app = make_app(vec![make_entry("src/ui/mod.rs")]);
        app.state.show_help();

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert!(app.state.is_help_visible());
        assert!(!app.state.is_switch_dialog_visible());
    }

    #[test]
    fn test_esc_in_switch_dialog_hides_it() {
        let mut app = make_app(vec![make_entry("src/ui/mod.rs")]);
        app.state.show_switch_dialog(make_dialog());
        assert!(app.state.is_switch_dialog_visible());

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert!(!app.state.is_switch_dialog_visible());
    }

    #[test]
    fn test_unknown_key_in_switch_dialog_keeps_it_open() {
        let mut app = make_app(vec![make_entry("src/ui/mod.rs")]);
        app.state.show_switch_dialog(make_dialog());

        let should_quit = app
            .handle_key_event(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE), None)
            .unwrap();

        assert!(!should_quit);
        assert!(app.state.is_switch_dialog_visible());
    }

    #[test]
    fn handle_branch_change_sets_flash_and_updates_last_seen() {
        let mut app = make_app(Vec::new());
        app.last_seen_branch = Some("feat-1".to_string());
        // Seed stale PR error state to verify it gets cleared.
        app.state.set_pr_error("stale error from feat-1".into());
        assert!(
            app.state.pr_state().error.is_some(),
            "test setup: pr error should be set"
        );

        app.handle_branch_change("feat-2");

        assert_eq!(app.last_seen_branch.as_deref(), Some("feat-2"));
        assert!(
            app.state.pr_state().error.is_none(),
            "expected pr error cleared after branch rename"
        );
        assert!(
            app.state.pr_state().info.is_none(),
            "expected pr info cleared after branch rename"
        );
        assert!(
            !app.state.pr_state().loading,
            "expected pr loading cleared after branch rename"
        );
        let flash = app.state.flash_message();
        assert!(
            flash.is_some_and(|m| m.contains("feat-1") && m.contains("feat-2")),
            "expected rename flash, got {flash:?}"
        );
    }

    #[test]
    fn apply_status_triggers_branch_change_on_rename() {
        use crate::git::worker::StatusBundle;
        let mut app = make_app(Vec::new());

        // First bundle establishes baseline.
        let bundle1 = StatusBundle {
            branch: "feat-1".to_string(),
            ..Default::default()
        };
        app.apply_status(bundle1);
        assert_eq!(app.last_seen_branch.as_deref(), Some("feat-1"));
        assert!(
            app.state.flash_message().is_none(),
            "first bundle should not flash a rename"
        );

        // Second bundle on a different branch fires the rename hook.
        let bundle2 = StatusBundle {
            branch: "feat-2".to_string(),
            ..Default::default()
        };
        app.apply_status(bundle2);
        assert_eq!(app.last_seen_branch.as_deref(), Some("feat-2"));
        let flash = app.state.flash_message();
        assert!(
            flash.is_some_and(|m| m.contains("feat-1") && m.contains("feat-2")),
            "expected rename flash on second apply, got {flash:?}"
        );

        // Third bundle on the same branch is a no-op for rename.
        app.state.clear_flash_message();
        let bundle3 = StatusBundle {
            branch: "feat-2".to_string(),
            ..Default::default()
        };
        app.apply_status(bundle3);
        assert!(
            app.state.flash_message().is_none(),
            "same-branch bundle should not refire rename flash"
        );
    }
}
