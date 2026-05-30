use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::ViewMode;

/// Top-level application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Debounce interval in milliseconds (can be overridden by CLI)
    pub debounce_ms: u64,
    /// Display settings
    pub display: DisplayConfig,
    /// PR widget configuration
    pub pr: PrConfig,
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
            debounce_ms: 500,
            display: DisplayConfig::default(),
            pr: PrConfig::default(),
            edit_command: None,
            base_branch: None,
        }
    }
}

/// Display-related configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Flash the background of a file row when its diff stats change
    pub flash_on_change: bool,
    /// Duration in milliseconds for the flash effect
    pub flash_duration_ms: u64,
    /// Rows of context kept visible above and below the selected row in the
    /// file list. 0 disables the feature. Clamped to `(viewport - 1) / 2` at
    /// runtime by ratatui.
    pub scroll_padding: usize,
    /// The view mode perch starts in. One of `"normal"`, `"condensed"`, `"tree"`.
    pub default_view: ViewMode,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            flash_on_change: true,
            flash_duration_ms: 600,
            scroll_padding: 3,
            default_view: ViewMode::Normal,
        }
    }
}

/// PR widget configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrConfig {
    /// Whether the PR tab / polling is enabled
    pub enabled: bool,
}

impl Default for PrConfig {
    fn default() -> Self {
        Self { enabled: true }
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
    fn test_removed_keys_are_ignored_gracefully() {
        let dir = std::env::temp_dir().join("perch-test-config-removed-keys");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
debounce_ms = 250

[display]
context_lines = 9
flash_on_change = false

[pr]
enabled = false
show_labels = true
layout = "right"

[keys]
quit = "x"
up = "w"
"#,
        )
        .unwrap();

        // Removed keys are unknown to serde and silently ignored; known keys still apply.
        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.debounce_ms, 250);
        assert!(!config.display.flash_on_change);
        assert!(!config.pr.enabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.debounce_ms, 500);
        assert!(config.display.flash_on_change);
        assert_eq!(config.display.flash_duration_ms, 600);
        assert_eq!(config.display.scroll_padding, 3);
    }

    #[test]
    fn test_default_pr_config() {
        let pr = PrConfig::default();
        assert!(pr.enabled);
    }

    #[test]
    fn test_load_nonexistent_uses_defaults() {
        let config = AppConfig::load(Some(Path::new("/tmp/nonexistent-perch-config.toml")));
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.debounce_ms, 500);
    }

    #[test]
    fn test_load_valid_toml() {
        let dir = std::env::temp_dir().join("perch-test-config-simplified");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
[pr]
enabled = false
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert!(!config.pr.enabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_partial_fills_defaults() {
        let dir = std::env::temp_dir().join("perch-test-config-partial-simplified");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "edit_command = \"nvim\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.edit_command.as_deref(), Some("nvim"));
        // Unspecified fields use defaults
        assert_eq!(config.debounce_ms, 500);
        assert!(config.display.flash_on_change);
        assert!(config.pr.enabled);
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
    fn test_default_view_defaults_to_normal() {
        let config = AppConfig::default();
        assert_eq!(config.display.default_view, crate::state::ViewMode::Normal);
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
    fn test_load_default_view_normal() {
        let dir = std::env::temp_dir().join("perch-test-config-default-view-normal");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[display]\ndefault_view = \"normal\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.display.default_view, crate::state::ViewMode::Normal);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_default_view_condensed() {
        let dir = std::env::temp_dir().join("perch-test-config-default-view-condensed");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[display]\ndefault_view = \"condensed\"\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(
            config.display.default_view,
            crate::state::ViewMode::Condensed
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rejects_old_view_names() {
        let dir = std::env::temp_dir().join("perch-test-config-rejects-old-view-names");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[display]\ndefault_view = \"expanded\"\n").unwrap();

        assert!(AppConfig::load(Some(&path)).is_err());

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
