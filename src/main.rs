#![allow(dead_code)]

mod actions;
mod app;
mod config;
mod git;
mod state;
mod ui;
mod watcher;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "git-rt",
    version,
    about = "Real-time terminal dashboard for git changes"
)]
struct Cli {
    /// Path to git repository (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Debounce interval in milliseconds
    #[arg(short, long, default_value_t = 200)]
    debounce: u64,

    /// Enable logging at the given level (trace, debug, info, warn, error)
    #[arg(long)]
    log: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing if requested — write to file since TUI owns stdout/stderr
    if let Some(ref level) = cli.log {
        let log_file = std::fs::File::create("/tmp/git-rt.log")
            .context("Failed to create log file at /tmp/git-rt.log")?;
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info")))
            .with_target(false)
            .with_writer(log_file)
            .with_ansi(false)
            .init();
    }

    let repo_path = cli
        .path
        .canonicalize()
        .context("Failed to resolve repository path")?;

    tracing::info!(?repo_path, "Starting git-rt");

    let config = config::AppConfig::load(cli.config.as_deref())?;

    let mut app = app::App::new(repo_path, config, cli.debounce)?;
    app.run()
}
