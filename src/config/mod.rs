use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ratatui::style::Color;
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
    /// User-defined color palette (compat shim — to be removed in Task 5/6)
    #[serde(default)]
    pub colors: HashMap<String, ColorValue>,
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
            colors: HashMap::new(),
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
    // ── Compat fields — to be removed in Task 5 ──────────────────────────────
    /// Vim-style format string for file rows
    pub file_line: String,
    /// Top and bottom statusline configuration
    pub statusline: StatusLineSectionConfig,
    /// Show expand marker (▼/space) before each file row
    pub show_expand_marker: bool,
    /// Padding around the file list area
    pub padding: PaddingConfig,
    /// Color theme configuration
    pub colors: ColorConfig,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            context_lines: 3,
            flash_on_change: true,
            flash_duration_ms: 600,
            file_line: "%s %f %- %+".to_string(),
            statusline: StatusLineSectionConfig::default(),
            show_expand_marker: true,
            padding: PaddingConfig::default(),
            colors: ColorConfig::default(),
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

// ── Compatibility shims ────────────────────────────────────────────────────────
// These types exist solely to keep ui/mod.rs and ui/status_format.rs compiling
// while they are rewritten in Task 5. They will be deleted in that task.

/// A color value that can be a named color or hex RGB string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColorValue(pub String);

impl ColorValue {
    /// Construct from a raw string.
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }

    /// Resolve to a ratatui `Color`.
    pub fn resolve(&self) -> Color {
        let s = self.0.trim();
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16);
                let g = u8::from_str_radix(&hex[2..4], 16);
                let b = u8::from_str_radix(&hex[4..6], 16);
                if let (Ok(r), Ok(g), Ok(b)) = (r, g, b) {
                    return Color::Rgb(r, g, b);
                }
            }
            return Color::Reset;
        }
        match s.to_lowercase().as_str() {
            "black" => Color::Black,
            "red" => Color::Red,
            "green" => Color::Green,
            "yellow" => Color::Yellow,
            "blue" => Color::Blue,
            "magenta" => Color::Magenta,
            "cyan" => Color::Cyan,
            "gray" | "grey" => Color::Gray,
            "darkgray" | "darkgrey" | "dark_gray" | "dark_grey" => Color::DarkGray,
            "lightred" | "light_red" => Color::LightRed,
            "lightgreen" | "light_green" => Color::LightGreen,
            "lightyellow" | "light_yellow" => Color::LightYellow,
            "lightblue" | "light_blue" => Color::LightBlue,
            "lightmagenta" | "light_magenta" => Color::LightMagenta,
            "lightcyan" | "light_cyan" => Color::LightCyan,
            "white" => Color::White,
            _ => Color::Reset,
        }
    }
}

/// Compat shim: single statusline bar config (to be removed in Task 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusLineConfig {
    pub status_line: String,
    pub foreground_color: ColorValue,
    pub background_color: ColorValue,
}

impl Default for StatusLineConfig {
    fn default() -> Self {
        Self {
            status_line: String::new(),
            foreground_color: ColorValue::new("white"),
            background_color: ColorValue::new("#1E1E1E"),
        }
    }
}

/// Compat shim: top/bottom statusline container (to be removed in Task 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusLineSectionConfig {
    pub top: StatusLineConfig,
    pub bottom: StatusLineConfig,
}

impl Default for StatusLineSectionConfig {
    fn default() -> Self {
        Self {
            top: StatusLineConfig::default(),
            bottom: StatusLineConfig {
                status_line: "%b  %c files  {red}%-{/} {green}%+{/}  %=%R".to_string(),
                ..StatusLineConfig::default()
            },
        }
    }
}

/// Compat shim: UI color palette (to be removed in Task 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiColors {
    pub selection_bg: ColorValue,
    pub selection_fg: ColorValue,
    pub flash_bg: ColorValue,
    pub empty_text: ColorValue,
}

impl Default for UiColors {
    fn default() -> Self {
        Self {
            selection_bg: ColorValue::new("darkgray"),
            selection_fg: ColorValue::new("white"),
            flash_bg: ColorValue::new("#64641E"),
            empty_text: ColorValue::new("darkgray"),
        }
    }
}

/// Compat shim: color config wrapper (to be removed in Task 5).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    pub ui: UiColors,
}

/// Compat shim: padding config (to be removed in Task 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PaddingConfig {
    pub top: u16,
    pub bottom: u16,
    pub left: u16,
    pub right: u16,
}

impl Default for PaddingConfig {
    fn default() -> Self {
        Self {
            top: 1,
            bottom: 0,
            left: 0,
            right: 2,
        }
    }
}

// ── End compatibility shims ───────────────────────────────────────────────────

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
