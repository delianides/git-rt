use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Theme name (e.g. "catppuccin-mocha", "dracula", "nord")
    pub theme: String,
    /// Debounce interval in milliseconds (can be overridden by CLI)
    pub debounce_ms: u64,
    /// Display settings
    pub display: DisplayConfig,
    /// PR widget configuration
    pub pr: PrConfig,
    /// Keybinding overrides
    pub keys: KeyConfig,
    /// Named actions that can be triggered on files
    pub actions: HashMap<String, ActionConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "catppuccin-mocha".to_string(),
            debounce_ms: 200,
            display: DisplayConfig::default(),
            pr: PrConfig::default(),
            keys: KeyConfig::default(),
            actions: HashMap::new(),
        }
    }
}

/// Display-related configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Number of diff context lines to show around changes
    pub context_lines: usize,
    /// Flash the background of a file row when its diff stats change
    pub flash_on_change: bool,
    /// Duration in milliseconds for the flash effect
    pub flash_duration_ms: u64,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            context_lines: 3,
            flash_on_change: true,
            flash_duration_ms: 600,
        }
    }
}

/// PR widget configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrConfig {
    /// Whether the PR widget is enabled
    pub enabled: bool,
    /// Layout mode: "bottom", "right", or "tab"
    pub layout: String,
    /// Whether to show PR labels
    pub show_labels: bool,
}

impl Default for PrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            layout: "bottom".to_string(),
            show_labels: false,
        }
    }
}

/// Keybinding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyConfig {
    pub quit: String,
    pub up: String,
    pub down: String,
    pub expand: String,
    pub collapse: String,
    pub refresh: String,
    /// How Enter behaves: "overlay" or "inline"
    pub enter: String,
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
            enter: "overlay".to_string(),
        }
    }
}

/// A configurable action that can be triggered on a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionConfig {
    /// Keybinding to trigger this action
    pub key: String,
    /// Command template to execute (supports {file} and {abs_file} placeholders)
    pub command: String,
}

impl AppConfig {
    /// Load config from a file path, or from the default XDG location,
    /// falling back to built-in defaults if no config file exists.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = path.map(PathBuf::from).or_else(|| {
            // Check ~/.config first (XDG convention, common on macOS for CLI tools)
            let xdg_path = dirs::home_dir()
                .map(|h| h.join(".config").join("git-rt").join("config.toml"))
                .filter(|p| p.exists());

            // Fall back to platform config dir (~/Library/Application Support on macOS)
            xdg_path.or_else(|| dirs::config_dir().map(|d| d.join("git-rt").join("config.toml")))
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

    /// Resolve the command for an action by substituting template variables.
    pub fn resolve_action_command(
        &self,
        action_name: &str,
        file_path: &str,
        abs_file_path: &str,
    ) -> Option<String> {
        let action = self.actions.get(action_name)?;

        Some(
            action
                .command
                .replace("{file}", file_path)
                .replace("{abs_file}", abs_file_path),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.theme, "catppuccin-mocha");
        assert_eq!(config.debounce_ms, 200);
        assert_eq!(config.display.context_lines, 3);
        assert!(config.display.flash_on_change);
        assert_eq!(config.display.flash_duration_ms, 600);
    }

    #[test]
    fn test_default_pr_config() {
        let pr = PrConfig::default();
        assert!(pr.enabled);
        assert_eq!(pr.layout, "bottom");
        assert!(!pr.show_labels);
    }

    #[test]
    fn test_default_keys() {
        let keys = KeyConfig::default();
        assert_eq!(keys.quit, "q");
        assert_eq!(keys.up, "k");
        assert_eq!(keys.down, "j");
        assert_eq!(keys.expand, "l");
        assert_eq!(keys.collapse, "h");
        assert_eq!(keys.refresh, "r");
        assert_eq!(keys.enter, "overlay");
    }

    #[test]
    fn test_load_nonexistent_uses_defaults() {
        let config = AppConfig::load(Some(Path::new("/tmp/nonexistent-git-rt-config.toml")));
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.debounce_ms, 200);
        assert_eq!(config.theme, "catppuccin-mocha");
    }

    #[test]
    fn test_load_valid_toml() {
        let dir = std::env::temp_dir().join("git-rt-test-config-simplified");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
theme = "dracula"

[pr]
enabled = false
layout = "right"

[keys]
enter = "inline"
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.theme, "dracula");
        assert!(!config.pr.enabled);
        assert_eq!(config.pr.layout, "right");
        assert_eq!(config.keys.enter, "inline");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_partial_fills_defaults() {
        let dir = std::env::temp_dir().join("git-rt-test-config-partial-simplified");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = \"nord\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.theme, "nord");
        // Unspecified fields use defaults
        assert_eq!(config.debounce_ms, 200);
        assert_eq!(config.display.context_lines, 3);
        assert!(config.display.flash_on_change);
        assert!(config.pr.enabled);
        assert_eq!(config.pr.layout, "bottom");
        assert_eq!(config.keys.enter, "overlay");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_resolve_action_command() {
        let mut config = AppConfig::default();
        config.actions.insert(
            "open_editor".to_string(),
            ActionConfig {
                key: "e".to_string(),
                command: "nvim {file}".to_string(),
            },
        );
        let cmd = config.resolve_action_command(
            "open_editor",
            "src/main.rs",
            "/home/user/repo/src/main.rs",
        );
        assert_eq!(cmd.unwrap(), "nvim src/main.rs");
    }

    #[test]
    fn test_resolve_action_unknown() {
        let config = AppConfig::default();
        let cmd = config.resolve_action_command("nonexistent", "file.rs", "/abs/file.rs");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_resolve_action_abs_file_template() {
        let mut config = AppConfig::default();
        config.actions.insert(
            "test_action".to_string(),
            ActionConfig {
                key: "t".to_string(),
                command: "open {abs_file}".to_string(),
            },
        );
        let cmd = config.resolve_action_command(
            "test_action",
            "src/main.rs",
            "/home/user/repo/src/main.rs",
        );
        assert_eq!(cmd.unwrap(), "open /home/user/repo/src/main.rs");
    }
}
