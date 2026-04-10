#![allow(dead_code)]

mod actions;
mod app;
mod config;
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

    /// Pin to a specific worktree (by name or path). Disables auto-follow.
    #[arg(long, conflicts_with = "branch")]
    worktree: Option<String>,

    /// Pin to the worktree with this branch checked out. Disables auto-follow.
    #[arg(long, conflicts_with = "worktree")]
    branch: Option<String>,
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

    // Resolve worktree/branch pinning
    let git_worktrees_dir = repo_path.join(".git").join("worktrees");
    let pinned_worktree = if let Some(ref wt_arg) = cli.worktree {
        Some(
            watcher::worktree::resolve_worktree_arg(&git_worktrees_dir, wt_arg)
                .with_context(|| format!("Failed to resolve --worktree '{wt_arg}'"))?,
        )
    } else if let Some(ref branch_arg) = cli.branch {
        Some(
            watcher::worktree::resolve_branch_arg(&git_worktrees_dir, branch_arg)
                .with_context(|| format!("Failed to resolve --branch '{branch_arg}'"))?,
        )
    } else {
        None
    };

    let (watch_path, auto_follow) = match pinned_worktree {
        Some(ref wt) => {
            tracing::info!(worktree = %wt.name, path = ?wt.path, "Pinned to worktree");
            (wt.path.clone(), false)
        }
        None => (repo_path.clone(), true),
    };

    let mut app = app::App::new(watch_path, repo_path, config, cli.debounce, auto_follow)?;
    app.run()
}
