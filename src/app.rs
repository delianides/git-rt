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

pub struct App {
    state: AppState,
    git: GitRepo,
    _watcher: FsWatcher,
    fs_rx: Receiver<()>,
    config: AppConfig,
    tick_rate: Duration,
}

impl App {
    pub fn new(repo_path: PathBuf, config: AppConfig, debounce_ms: u64) -> Result<Self> {
        let git = GitRepo::new(&repo_path).context("Failed to open git repository")?;

        // Initial git status computation
        let files = git.status()?;

        let flash_duration = Duration::from_millis(config.display.flash_duration_ms);
        let state = AppState::new(files, flash_duration);

        let (fs_rx, watcher) =
            FsWatcher::new(&repo_path, Duration::from_millis(debounce_ms))?;

        Ok(Self {
            state,
            git,
            _watcher: watcher,
            fs_rx,
            config,
            tick_rate: Duration::from_millis(250),
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
            terminal.draw(&self.state, &self.config.display)?;

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
        tracing::debug!(file_count = files.len(), "Git status returned");
        for f in &files {
            tracing::debug!(path = %f.path, ins = f.insertions, del = f.deletions, "  file");
        }
        self.state.update_files(files);
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
}
