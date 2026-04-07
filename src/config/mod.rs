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
            actions: HashMap::new(),
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
    /// Show refresh counter and last-updated time in the status bar
    pub show_refresh_counter: bool,
    /// Flash the background of a file row when its diff stats change
    pub flash_on_change: bool,
    /// Duration in milliseconds for the flash effect
    pub flash_duration_ms: u64,
    /// Vim-style format string for file rows (e.g. "%s %f %- %+")
    pub file_line: String,
    /// Show expand marker (▼/space) before each file row
    pub show_expand_marker: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            show_status: true,
            context_lines: 3,
            show_refresh_counter: false,
            flash_on_change: true,
            flash_duration_ms: 600,
            file_line: "%s %f %- %+".to_string(),
            show_expand_marker: true,
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

    /// Resolve the command for an action by substituting template variables
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
        assert_eq!(config.debounce_ms, 200);
        assert!(config.display.show_status);
        assert_eq!(config.display.context_lines, 3);
        assert!(!config.display.show_refresh_counter);
        assert!(config.display.flash_on_change);
        assert_eq!(config.display.flash_duration_ms, 600);
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
    }

    #[test]
    fn test_default_actions_empty() {
        let config = AppConfig::default();
        assert!(config.actions.is_empty());
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
    fn test_resolve_action_command_unknown() {
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

    #[test]
    fn test_load_nonexistent_config_uses_defaults() {
        let config = AppConfig::load(Some(Path::new("/tmp/nonexistent-git-rt-config.toml")));
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.debounce_ms, 200);
    }

    #[test]
    fn test_load_valid_toml() {
        let dir = std::env::temp_dir().join("git-rt-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
debounce_ms = 500

[display]
show_status = false
flash_on_change = true
flash_duration_ms = 1000
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.debounce_ms, 500);
        assert!(!config.display.show_status);
        assert!(config.display.flash_on_change);
        assert_eq!(config.display.flash_duration_ms, 1000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_partial_toml_fills_defaults() {
        let dir = std::env::temp_dir().join("git-rt-test-config-partial");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "debounce_ms = 100\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.debounce_ms, 100);
        // Defaults should fill in
        assert!(config.display.show_status);
        assert_eq!(config.display.context_lines, 3);
        assert!(config.display.flash_on_change);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_default_file_line_format() {
        let config = DisplayConfig::default();
        assert_eq!(config.file_line, "%s %f %- %+");
        assert!(config.show_expand_marker);
    }

    #[test]
    fn test_file_line_from_toml() {
        let dir = std::env::temp_dir().join("git-rt-test-file-line");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
[display]
file_line = "%f %g"
show_expand_marker = false
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.display.file_line, "%f %g");
        assert!(!config.display.show_expand_marker);

        std::fs::remove_dir_all(&dir).ok();
    }
}
