use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Debounce interval in milliseconds (can be overridden by CLI)
    pub debounce_ms: u64,

    /// Display settings
    pub display: DisplayConfig,

    /// Keybinding overrides
    pub keys: KeyConfig,

    /// Named actions that can be triggered on files
    pub actions: HashMap<String, ActionConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 200,
            display: DisplayConfig::default(),
            keys: KeyConfig::default(),
            actions: default_actions(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Show file status character (M, A, D, etc.)
    pub show_status: bool,
    /// Maximum number of diff context lines to show around changes
    pub context_lines: usize,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            show_status: true,
            context_lines: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyConfig {
    pub quit: String,
    pub up: String,
    pub down: String,
    pub expand: String,
    pub collapse: String,
    pub refresh: String,
}

impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            quit: "q".to_string(),
            up: "k".to_string(),
            down: "j".to_string(),
            expand: "l".to_string(),
            collapse: "h".to_string(),
            refresh: "r".to_string(),
        }
    }
}

/// A configurable action that can be triggered on a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionConfig {
    /// Keybinding to trigger this action
    pub key: String,
    /// Command template for tmux environment
    pub tmux: Option<String>,
    /// Command template for zellij environment
    pub zellij: Option<String>,
    /// Command template for wezterm environment
    pub wezterm: Option<String>,
    /// Fallback command for plain terminal
    pub fallback: Option<String>,
}

/// Detected terminal multiplexer environment
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Multiplexer {
    Tmux,
    Zellij,
    Wezterm,
    None,
}

impl Multiplexer {
    /// Detect the current multiplexer from environment variables
    pub fn detect() -> Self {
        if std::env::var("TMUX").is_ok() {
            Self::Tmux
        } else if std::env::var("ZELLIJ").is_ok() {
            Self::Zellij
        } else if std::env::var("WEZTERM_PANE").is_ok() {
            Self::Wezterm
        } else {
            Self::None
        }
    }
}

impl AppConfig {
    /// Load config from a file path, or from the default XDG location,
    /// falling back to built-in defaults if no config file exists.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = path
            .map(PathBuf::from)
            .or_else(|| {
                dirs::config_dir().map(|d| d.join("git-rt").join("config.toml"))
            });

        match config_path {
            Some(ref p) if p.exists() => {
                let contents = std::fs::read_to_string(p)
                    .with_context(|| format!("Failed to read config file: {}", p.display()))?;
                let config: AppConfig = toml::from_str(&contents)
                    .with_context(|| format!("Failed to parse config file: {}", p.display()))?;
                tracing::info!(?p, "Loaded config");
                Ok(config)
            }
            _ => {
                tracing::debug!("No config file found, using defaults");
                Ok(AppConfig::default())
            }
        }
    }

    /// Resolve the command for an action given the current multiplexer
    pub fn resolve_action_command(
        &self,
        action_name: &str,
        file_path: &str,
        abs_file_path: &str,
        mux: &Multiplexer,
    ) -> Option<String> {
        let action = self.actions.get(action_name)?;

        let template = match mux {
            Multiplexer::Tmux => action.tmux.as_ref().or(action.fallback.as_ref()),
            Multiplexer::Zellij => action.zellij.as_ref().or(action.fallback.as_ref()),
            Multiplexer::Wezterm => action.wezterm.as_ref().or(action.fallback.as_ref()),
            Multiplexer::None => action.fallback.as_ref(),
        }?;

        Some(
            template
                .replace("{file}", file_path)
                .replace("{abs_file}", abs_file_path),
        )
    }
}

/// Built-in default actions
fn default_actions() -> HashMap<String, ActionConfig> {
    let mut actions = HashMap::new();

    actions.insert(
        "open_editor".to_string(),
        ActionConfig {
            key: "e".to_string(),
            tmux: Some("tmux split-window -h 'nvim {file}'".to_string()),
            zellij: Some("zellij run --direction right -- nvim {file}".to_string()),
            wezterm: Some("wezterm cli split-pane --right -- nvim {file}".to_string()),
            fallback: Some("nvim {file}".to_string()),
        },
    );

    actions.insert(
        "diff_view".to_string(),
        ActionConfig {
            key: "d".to_string(),
            tmux: Some("tmux popup -w 80% -h 80% 'git diff -- {file} | delta'".to_string()),
            zellij: Some(
                "zellij run --floating -- sh -c 'git diff -- {file} | delta'".to_string(),
            ),
            wezterm: None,
            fallback: Some("git diff -- {file} | delta".to_string()),
        },
    );

    actions
}
