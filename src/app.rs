use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use crossterm::event::{
    self, DisableFocusChange, EnableFocusChange, Event as TermEvent, KeyCode, KeyModifiers,
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
    /// Whether to auto-follow the most recently active worktree
    auto_follow: bool,
    /// Receiver for GitHub PR events
    gh_rx: Option<Receiver<crate::github::GitHubEvent>>,
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
            auto_follow,
            gh_rx,
            base_override,
            worker_tx,
            worker_rx,
            worker_handle: Some(worker_handle),
        })
    }

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
                match resp {
                    Response::Status(bundle) => self.apply_status(*bundle),
                    Response::SwitchAck(_) => {
                        // Handled inline by the SwitchRepo caller in Task 7.
                    }
                    Response::Error(msg) => {
                        tracing::warn!(error = %msg, "Worker error");
                        self.state.set_computing(false);
                    }
                }
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
                self.check_worktree_activity()?;
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

                    // Expand / open diff (Enter, l, Right, Space, d)
                    (_, KeyCode::Enter)
                    | (_, KeyCode::Char('l'))
                    | (_, KeyCode::Right)
                    | (_, KeyCode::Char(' '))
                    | (_, KeyCode::Char('d')) => {
                        self.open_pager_diff(terminal)?;
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

    /// Send a Recompute request to the worker. Non-blocking. The response
    /// is applied later when the event loop drains `worker_rx` (Task 5).
    fn handle_fs_change(&mut self) -> Result<()> {
        tracing::debug!("Filesystem change detected; sending Recompute to worker");
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
        let _ = self.worker_tx.try_send(Request::Recompute);
        self.state.set_computing(true);

        Ok(())
    }

    /// On every tick, check which worktree was most recently active (by HEAD +
    /// index mtime) and switch to it if it differs from the current watch path.
    fn check_worktree_activity(&mut self) -> Result<()> {
        if !self.auto_follow {
            return Ok(());
        }
        let worktrees = crate::watcher::activity::list_all_worktrees(&self.repo_path);
        if worktrees.len() <= 1 {
            return Ok(());
        }
        let newest = worktrees
            .iter()
            .filter_map(|wt| {
                let activity = crate::watcher::activity::worktree_last_activity(&wt.path)?;
                Some((wt, activity))
            })
            .max_by_key(|(_, mtime)| *mtime);

        if let Some((wt, _)) = newest {
            if wt.path != self.watch_path {
                tracing::info!(
                    worktree = %wt.name,
                    path = ?wt.path,
                    "Switching to most recently active worktree"
                );
                let name = wt.name.clone();
                self.switch_to_path(wt.path.clone())?;
                self.state
                    .set_flash_message(format!("Switched to worktree: {name}"));
            }
        }
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
