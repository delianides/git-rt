#![allow(dead_code)]

mod app;
mod config;
mod fuzzy;
mod git;
mod github;
mod state;
mod theme;
mod ui;
mod watcher;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "perch",
    version,
    about = "Real-time terminal dashboard for git changes"
)]
struct Cli {
    /// Path inside a git repository (defaults to current directory; repo root is discovered)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Debounce interval in milliseconds
    #[arg(short, long, default_value_t = 500)]
    debounce: u64,

    /// Enable logging at the given level (trace, debug, info, warn, error)
    #[arg(long)]
    log: Option<String>,

    /// Base branch for branch-scoped diff (overrides config).
    /// Auto-detected from remote if omitted.
    #[arg(long)]
    base: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing if requested — write to file since TUI owns stdout/stderr
    if let Some(ref level) = cli.log {
        let log_file = std::fs::File::create("/tmp/perch.log")
            .context("Failed to create log file at /tmp/perch.log")?;
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info")))
            .with_target(false)
            .with_writer(log_file)
            .with_ansi(false)
            .init();
    }

    let startup_t0 = std::time::Instant::now();

    let t = std::time::Instant::now();
    let launch_path = cli
        .path
        .canonicalize()
        .context("Failed to resolve launch path")?;
    tracing::debug!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        "startup: canonicalize launch path"
    );

    let t = std::time::Instant::now();
    let repo_path = git::discover_worktree_root(&launch_path)
        .with_context(|| format!("Launch path: {}", launch_path.display()))?;
    tracing::debug!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        "startup: discover_worktree_root"
    );

    tracing::info!(?repo_path, "Starting perch");

    let t = std::time::Instant::now();
    let config = config::AppConfig::load(cli.config.as_deref())?;
    tracing::debug!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        "startup: config load"
    );

    let watch_path = repo_path.clone();

    let t = std::time::Instant::now();
    let mut app = app::App::new(watch_path, repo_path, config, cli.debounce, cli.base)?;
    tracing::debug!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        "startup: App::new"
    );
    tracing::info!(
        elapsed_ms = startup_t0.elapsed().as_millis() as u64,
        "startup: total before run()"
    );
    app.run()
}
