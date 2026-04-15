#![allow(dead_code)]

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

    /// Pin to the worktree (main or linked) with this branch checked out as the
    /// starting worktree. Does not disable auto-follow — use --no-follow for that.
    #[arg(long)]
    branch: Option<String>,

    /// Disable automatic worktree following. When set, git-rt will stay on the
    /// worktree it was launched in (or pinned to) and will not switch to other
    /// worktrees even if activity is detected elsewhere. Also disables cold-start
    /// activity scan.
    #[arg(long)]
    no_follow: bool,

    /// Theme name or path to a theme file (TOML or JSON).
    /// Overrides the theme set in the config file.
    #[arg(long)]
    theme: Option<String>,

    /// Base branch for branch-scoped diff (overrides config).
    /// Auto-detected from remote if omitted.
    #[arg(long)]
    base: Option<String>,
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

    let launch_path = cli
        .path
        .canonicalize()
        .context("Failed to resolve launch path")?;
    let repo_path = git::discover_worktree_root(&launch_path)
        .with_context(|| format!("Launch path: {}", launch_path.display()))?;

    tracing::info!(?repo_path, "Starting git-rt");

    let config = config::AppConfig::load(cli.config.as_deref())?;

    // Resolve branch pinning: search across main + all linked worktrees.
    let pinned_worktree = if let Some(ref branch_arg) = cli.branch {
        Some(
            watcher::activity::resolve_branch_arg(&repo_path, branch_arg)
                .with_context(|| format!("Failed to resolve --branch '{branch_arg}'"))?,
        )
    } else {
        None
    };

    // auto_follow is now controlled only by --no-follow.
    // --branch no longer disables auto-follow.
    let auto_follow = !cli.no_follow;

    let watch_path = match pinned_worktree {
        Some(ref wt) => {
            tracing::info!(worktree = %wt.name, path = ?wt.path, "Pinned to worktree");
            wt.path.clone()
        }
        None => {
            if auto_follow {
                cold_start_pick(&repo_path)
            } else {
                // --no-follow without pinning: stay on launch directory
                repo_path.clone()
            }
        }
    };

    let mut app = app::App::new(
        watch_path,
        repo_path,
        config,
        cli.debounce,
        auto_follow,
        cli.theme,
        cli.base,
    )?;
    app.run()
}

/// Scan all worktrees (main + linked) and return the path of the one with
/// the most recent activity. Falls back to `repo_path` if no worktrees are
/// found or ranking fails.
fn cold_start_pick(repo_path: &std::path::Path) -> PathBuf {
    let worktrees = watcher::activity::list_all_worktrees(repo_path);
    if worktrees.is_empty() {
        return repo_path.to_path_buf();
    }

    let ranked = watcher::activity::rank_by_activity(&worktrees);
    match ranked.first() {
        Some(winner) => {
            tracing::info!(
                worktree = %winner.info.name,
                path = ?winner.info.path,
                last_activity = ?winner.last_activity,
                "Cold-start auto-switched to most active worktree"
            );
            winner.info.path.clone()
        }
        None => repo_path.to_path_buf(),
    }
}
