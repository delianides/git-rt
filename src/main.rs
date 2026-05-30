#![allow(dead_code)]

mod app;
mod config;
mod fuzzy;
mod git;
mod github;
mod state;
mod ui;
mod watcher;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Version reported by `--version`. Debug builds append the git describe
/// string (`-dev.<sha>[.dirty]`); release builds report the clean crate
/// version. `PERCH_GIT_DESCRIBE` is set by build.rs.
const VERSION: &str = if cfg!(debug_assertions) {
    concat!(
        env!("CARGO_PKG_VERSION"),
        "-dev.",
        env!("PERCH_GIT_DESCRIBE")
    )
} else {
    env!("CARGO_PKG_VERSION")
};

#[derive(Parser, Debug)]
#[command(
    name = "perch",
    version = VERSION,
    about = "Real-time terminal dashboard for git changes"
)]
struct Cli {
    /// Path inside a git repository (defaults to current directory; repo root is discovered)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Debounce interval in milliseconds (overrides config)
    #[arg(short, long)]
    debounce: Option<u64>,

    /// Enable logging at the given level (trace, debug, info, warn, error)
    #[arg(long)]
    log: Option<String>,

    /// Base branch for branch-scoped diff (overrides config).
    /// Auto-detected if omitted.
    #[arg(long)]
    base: Option<String>,

    /// Disable the GitHub PR status strip (overrides config)
    #[arg(long)]
    no_pr: bool,

    /// Startup view mode (overrides config)
    #[arg(long, value_enum)]
    view: Option<crate::state::ViewMode>,

    /// Disable the row flash on change (overrides config)
    #[arg(long)]
    no_flash: bool,

    /// Flash duration in milliseconds (overrides config)
    #[arg(long)]
    flash_duration: Option<u64>,

    /// Rows of context kept above/below the selection (overrides config)
    #[arg(long)]
    scroll_padding: Option<usize>,

    /// Shell command used to open a file for editing (overrides config)
    #[arg(long)]
    edit_command: Option<String>,
}

/// Overlay CLI flags onto a loaded config. Precedence: CLI flag > config > default.
/// Value flags override only when present; `--no-pr`/`--no-flash` only force off.
fn merge_cli_overrides(mut config: config::AppConfig, cli: &Cli) -> config::AppConfig {
    if let Some(d) = cli.debounce {
        config.debounce_ms = d;
    }
    if let Some(b) = cli.base.clone() {
        config.base_branch = Some(b);
    }
    if cli.no_pr {
        config.pr.enabled = false;
    }
    if let Some(v) = cli.view {
        config.display.default_view = v;
    }
    if cli.no_flash {
        config.display.flash_on_change = false;
    }
    if let Some(ms) = cli.flash_duration {
        config.display.flash_duration_ms = ms;
    }
    if let Some(n) = cli.scroll_padding {
        config.display.scroll_padding = n;
    }
    if let Some(cmd) = cli.edit_command.clone() {
        config.edit_command = Some(cmd);
    }
    config
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
    let config = merge_cli_overrides(config, &cli);
    tracing::debug!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        "startup: config load"
    );

    let watch_path = repo_path.clone();

    let t = std::time::Instant::now();
    let mut app = app::App::new(watch_path, repo_path, config)?;
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

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_has_dev_suffix_in_debug_builds() {
        // `cargo test` compiles with debug_assertions enabled, so VERSION must
        // carry the dev describe suffix and still start with the crate version.
        assert!(
            VERSION.starts_with(env!("CARGO_PKG_VERSION")),
            "got {VERSION}"
        );
        assert!(VERSION.contains("-dev."), "got {VERSION}");
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::state::ViewMode;
    use std::path::PathBuf;

    fn bare_cli() -> Cli {
        Cli {
            path: PathBuf::from("."),
            config: None,
            debounce: None,
            log: None,
            base: None,
            no_pr: false,
            view: None,
            no_flash: false,
            flash_duration: None,
            scroll_padding: None,
            edit_command: None,
        }
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn absent_flags_keep_config_values() {
        let mut config = AppConfig::default();
        config.debounce_ms = 250;
        config.display.default_view = ViewMode::Tree;
        let merged = merge_cli_overrides(config, &bare_cli());
        assert_eq!(merged.debounce_ms, 250);
        assert_eq!(merged.display.default_view, ViewMode::Tree);
        assert!(merged.pr.enabled);
        assert!(merged.display.flash_on_change);
    }

    #[test]
    fn value_flags_override_config() {
        let config = AppConfig::default();
        let cli = Cli {
            debounce: Some(900),
            base: Some("develop".to_string()),
            view: Some(ViewMode::Condensed),
            flash_duration: Some(123),
            scroll_padding: Some(7),
            edit_command: Some("nano".to_string()),
            ..bare_cli()
        };
        let merged = merge_cli_overrides(config, &cli);
        assert_eq!(merged.debounce_ms, 900);
        assert_eq!(merged.base_branch.as_deref(), Some("develop"));
        assert_eq!(merged.display.default_view, ViewMode::Condensed);
        assert_eq!(merged.display.flash_duration_ms, 123);
        assert_eq!(merged.display.scroll_padding, 7);
        assert_eq!(merged.edit_command.as_deref(), Some("nano"));
    }

    #[test]
    fn no_pr_and_no_flash_force_off() {
        let config = AppConfig::default(); // both default true
        let cli = Cli {
            no_pr: true,
            no_flash: true,
            ..bare_cli()
        };
        let merged = merge_cli_overrides(config, &cli);
        assert!(!merged.pr.enabled);
        assert!(!merged.display.flash_on_change);
    }
}
