use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::ViewMode;

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
    /// Shell command used to open a file for editing. Falls back to `$EDITOR`,
    /// then to `vim` when unset.
    pub edit_command: Option<String>,
    /// Base branch for branch-scoped diff (e.g. "main", "develop").
    /// Auto-detected from remote if omitted.
    pub base_branch: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "catppuccin-mocha".to_string(),
            debounce_ms: 500,
            display: DisplayConfig::default(),
            pr: PrConfig::default(),
            keys: KeyConfig::default(),
            edit_command: None,
            base_branch: None,
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
    /// Rows of context kept visible above and below the selected row in the
    /// file list. 0 disables the feature. Clamped to `(viewport - 1) / 2` at
    /// runtime by ratatui.
    pub scroll_padding: usize,
    /// The view mode perch starts in. One of `"flat"`, `"tree"`, `"expanded"`.
    pub default_view: ViewMode,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            context_lines: 3,
            flash_on_change: true,
            flash_duration_ms: 600,
            scroll_padding: 3,
            default_view: ViewMode::Expanded,
        }
    }
}

/// PR widget configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrConfig {
    /// Whether the PR tab / polling is enabled
    pub enabled: bool,
    /// Whether to show PR labels
    pub show_labels: bool,
    /// **Deprecated**: retained so old config files parse cleanly. No longer
    /// affects rendering since the PR widget is now a tab.
    #[serde(default)]
    pub layout: Option<String>,
}

impl Default for PrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_labels: false,
            layout: None,
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

impl AppConfig {
    /// Load config from a file path, or from the default XDG location,
    /// falling back to built-in defaults if no config file exists.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = path.map(PathBuf::from).or_else(|| {
            // Check ~/.config first (XDG convention, common on macOS for CLI tools)
            let xdg_path = dirs::home_dir()
                .map(|h| h.join(".config").join("perch").join("config.toml"))
                .filter(|p| p.exists());

            // Fall back to platform config dir (~/Library/Application Support on macOS)
            xdg_path.or_else(|| dirs::config_dir().map(|d| d.join("perch").join("config.toml")))
        });

        match config_path {
            Some(ref p) if p.exists() => {
                let contents = std::fs::read_to_string(p)
                    .with_context(|| format!("Failed to read config file: {}", p.display()))?;
                let config: AppConfig = toml::from_str(&contents)
                    .with_context(|| format!("Failed to parse config file: {}", p.display()))?;
                tracing::info!(?p, "Loaded config");

                if config.pr.layout.is_some() {
                    tracing::warn!(
                        "`pr.layout` is deprecated and ignored — the PR widget is now a tab."
                    );
                }

                Ok(config)
            }
            _ => {
                tracing::debug!("No config file found, using defaults");
                Ok(AppConfig::default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.theme, "catppuccin-mocha");
        assert_eq!(config.debounce_ms, 500);
        assert_eq!(config.display.context_lines, 3);
        assert!(config.display.flash_on_change);
        assert_eq!(config.display.flash_duration_ms, 600);
        assert_eq!(config.display.scroll_padding, 3);
    }

    #[test]
    fn test_default_pr_config() {
        let pr = PrConfig::default();
        assert!(pr.enabled);
        assert!(pr.layout.is_none());
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
    }

    #[test]
    fn test_load_nonexistent_uses_defaults() {
        let config = AppConfig::load(Some(Path::new("/tmp/nonexistent-perch-config.toml")));
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.debounce_ms, 500);
        assert_eq!(config.theme, "catppuccin-mocha");
    }

    #[test]
    fn test_load_valid_toml() {
        let dir = std::env::temp_dir().join("perch-test-config-simplified");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
theme = "dracula"

[pr]
enabled = false
layout = "right"
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.theme, "dracula");
        assert!(!config.pr.enabled);
        assert_eq!(config.pr.layout.as_deref(), Some("right"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_partial_fills_defaults() {
        let dir = std::env::temp_dir().join("perch-test-config-partial-simplified");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = \"nord\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.theme, "nord");
        // Unspecified fields use defaults
        assert_eq!(config.debounce_ms, 500);
        assert_eq!(config.display.context_lines, 3);
        assert!(config.display.flash_on_change);
        assert!(config.pr.enabled);
        assert!(config.pr.layout.is_none());
        assert_eq!(config.display.scroll_padding, 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_scroll_padding_zero() {
        let dir = std::env::temp_dir().join("perch-test-config-scroll-padding-zero");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[display]\nscroll_padding = 0\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.display.scroll_padding, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_scroll_padding_custom() {
        let dir = std::env::temp_dir().join("perch-test-config-scroll-padding-custom");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[display]\nscroll_padding = 10\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.display.scroll_padding, 10);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_default_view_defaults_to_expanded() {
        let config = AppConfig::default();
        assert_eq!(
            config.display.default_view,
            crate::state::ViewMode::Expanded
        );
    }

    #[test]
    fn test_load_default_view_tree() {
        let dir = std::env::temp_dir().join("perch-test-config-default-view-tree");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[display]\ndefault_view = \"tree\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.display.default_view, crate::state::ViewMode::Tree);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_base_branch_config() {
        let dir = std::env::temp_dir().join("perch-test-config-base-branch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "base_branch = \"develop\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.base_branch.as_deref(), Some("develop"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_base_branch_default_none() {
        let config = AppConfig::default();
        assert!(config.base_branch.is_none());
    }

    #[test]
    fn test_legacy_pr_layout_parses_without_error() {
        let dir = std::env::temp_dir().join("perch-test-config-legacy-layout");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
[pr]
enabled = true
layout = "right"
show_labels = false
"#,
        )
        .unwrap();

        // Legacy `layout` key should parse cleanly — accepted but deprecated.
        let config = AppConfig::load(Some(&path)).unwrap();
        assert!(config.pr.enabled);
        assert!(!config.pr.show_labels);
        // The deprecated field is still present in the struct but its value is
        // irrelevant to runtime behavior.
        assert_eq!(config.pr.layout.as_deref(), Some("right"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_command_round_trip() {
        let toml = r#"edit_command = "nvim -p""#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.edit_command.as_deref(), Some("nvim -p"));
    }

    #[test]
    fn edit_command_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.edit_command.is_none());
    }
}
