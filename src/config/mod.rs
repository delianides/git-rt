use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// A color value that can be a named color or hex RGB string.
/// Resolves to a `ratatui::style::Color` at render time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColorValue(String);

impl ColorValue {
    /// Construct a `ColorValue` from a raw string (named color or hex).
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }

    /// Resolve the color string to a ratatui Color.
    /// Supports named colors (case-insensitive) and hex RGB (#RRGGBB).
    /// Falls back to Color::Reset for invalid values.
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    pub ui: UiColors,
}

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

/// Configuration for a single statusbar (top or bottom)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarConfig {
    /// Format string for this bar. Empty string hides the bar.
    pub status_line: String,
    /// Foreground color for unstyled text in this bar
    pub foreground_color: ColorValue,
    /// Background color for the entire bar
    pub background_color: ColorValue,
}

/// Container for top and bottom statusbar configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarSectionConfig {
    pub top: StatusBarConfig,
    pub bottom: StatusBarConfig,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            status_line: String::new(),
            foreground_color: ColorValue::new("white"),
            background_color: ColorValue::new("#1E1E1E"),
        }
    }
}

impl Default for StatusBarSectionConfig {
    fn default() -> Self {
        Self {
            top: StatusBarConfig::default(),
            bottom: StatusBarConfig {
                status_line: "%b  %c files  {red}%-{/} {green}%+{/}  %=%R".to_string(),
                ..StatusBarConfig::default()
            },
        }
    }
}

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

    /// User-defined color palette for statusbar format tags
    #[serde(default)]
    pub colors: HashMap<String, ColorValue>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 200,
            display: DisplayConfig::default(),
            keys: KeyConfig::default(),
            actions: HashMap::new(),
            colors: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
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
    /// Top and bottom statusbar configuration
    pub statusbar: StatusBarSectionConfig,
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
            show_refresh_counter: false,
            flash_on_change: true,
            flash_duration_ms: 600,
            file_line: "%s %f %- %+".to_string(),
            statusbar: StatusBarSectionConfig::default(),
            show_expand_marker: true,
            padding: PaddingConfig::default(),
            colors: ColorConfig::default(),
        }
    }
}

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
        assert_eq!(config.display.context_lines, 3);
        assert!(!config.display.show_refresh_counter);
        assert!(config.display.flash_on_change);
        assert_eq!(config.display.flash_duration_ms, 600);
        assert_eq!(config.display.padding.top, 1);
        assert_eq!(config.display.padding.bottom, 0);
        assert_eq!(config.display.padding.left, 0);
        assert_eq!(config.display.padding.right, 2);
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
flash_on_change = true
flash_duration_ms = 1000
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.debounce_ms, 500);
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

    #[test]
    fn test_color_value_named_red() {
        let cv = ColorValue::new("red");
        assert_eq!(cv.resolve(), Color::Red);
    }

    #[test]
    fn test_color_value_named_green() {
        let cv = ColorValue::new("green");
        assert_eq!(cv.resolve(), Color::Green);
    }

    #[test]
    fn test_color_value_named_yellow() {
        let cv = ColorValue::new("yellow");
        assert_eq!(cv.resolve(), Color::Yellow);
    }

    #[test]
    fn test_color_value_named_cyan() {
        let cv = ColorValue::new("cyan");
        assert_eq!(cv.resolve(), Color::Cyan);
    }

    #[test]
    fn test_color_value_named_magenta() {
        let cv = ColorValue::new("magenta");
        assert_eq!(cv.resolve(), Color::Magenta);
    }

    #[test]
    fn test_color_value_named_white() {
        let cv = ColorValue::new("white");
        assert_eq!(cv.resolve(), Color::White);
    }

    #[test]
    fn test_color_value_named_darkgray() {
        let cv = ColorValue::new("darkgray");
        assert_eq!(cv.resolve(), Color::DarkGray);
    }

    #[test]
    fn test_color_value_underscore_variants() {
        assert_eq!(ColorValue::new("dark_gray").resolve(), Color::DarkGray);
        assert_eq!(ColorValue::new("light_red").resolve(), Color::LightRed);
        assert_eq!(ColorValue::new("light_green").resolve(), Color::LightGreen);
        assert_eq!(ColorValue::new("light_blue").resolve(), Color::LightBlue);
        assert_eq!(
            ColorValue::new("light_yellow").resolve(),
            Color::LightYellow
        );
        assert_eq!(
            ColorValue::new("light_magenta").resolve(),
            Color::LightMagenta
        );
        assert_eq!(ColorValue::new("light_cyan").resolve(), Color::LightCyan);
    }

    #[test]
    fn test_color_value_named_case_insensitive() {
        let cv = ColorValue::new("Red");
        assert_eq!(cv.resolve(), Color::Red);
    }

    #[test]
    fn test_color_value_hex() {
        let cv = ColorValue::new("#FF5733");
        assert_eq!(cv.resolve(), Color::Rgb(255, 87, 51));
    }

    #[test]
    fn test_color_value_hex_lowercase() {
        let cv = ColorValue::new("#ff5733");
        assert_eq!(cv.resolve(), Color::Rgb(255, 87, 51));
    }

    #[test]
    fn test_color_value_hex_black() {
        let cv = ColorValue::new("#000000");
        assert_eq!(cv.resolve(), Color::Rgb(0, 0, 0));
    }

    #[test]
    fn test_color_value_hex_white() {
        let cv = ColorValue::new("#FFFFFF");
        assert_eq!(cv.resolve(), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_color_value_invalid_fallback() {
        let cv = ColorValue::new("notacolor");
        assert_eq!(cv.resolve(), Color::Reset);
    }

    #[test]
    fn test_color_value_invalid_hex_fallback() {
        let cv = ColorValue::new("#ZZZZZZ");
        assert_eq!(cv.resolve(), Color::Reset);
    }

    #[test]
    fn test_color_value_empty_fallback() {
        let cv = ColorValue::new("");
        assert_eq!(cv.resolve(), Color::Reset);
    }

    #[test]
    fn test_ui_colors_defaults() {
        let colors = UiColors::default();
        assert_eq!(colors.selection_bg.resolve(), Color::DarkGray);
        assert_eq!(colors.selection_fg.resolve(), Color::White);
        assert_eq!(colors.flash_bg.resolve(), Color::Rgb(100, 100, 30));
        assert_eq!(colors.empty_text.resolve(), Color::DarkGray);
    }

    #[test]
    fn test_color_config_on_display_config() {
        let display = DisplayConfig::default();
        assert_eq!(display.colors.ui.selection_bg.resolve(), Color::DarkGray);
    }

    #[test]
    fn test_toml_full_color_config() {
        let dir = std::env::temp_dir().join("git-rt-test-color-full");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r##"
[display.colors.ui]
selection_bg = "#444444"
selection_fg = "#EEEEEE"
flash_bg = "#665500"
empty_text = "#555555"
"##,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(
            config.display.colors.ui.selection_fg.resolve(),
            Color::Rgb(238, 238, 238)
        );
        assert_eq!(
            config.display.colors.ui.selection_bg.resolve(),
            Color::Rgb(68, 68, 68)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_toml_no_colors_uses_defaults() {
        let dir = std::env::temp_dir().join("git-rt-test-color-none");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "debounce_ms = 100\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(
            config.display.colors.ui.selection_bg.resolve(),
            Color::DarkGray
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_statusbar_from_toml() {
        let dir = std::env::temp_dir().join("git-rt-test-statusbar");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r##"
[display.statusbar.top]
status_line = "{dim}%h{/}"
foreground_color = "cyan"

[display.statusbar.bottom]
status_line = "%b %c"
background_color = "#222222"
"##,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.display.statusbar.top.status_line, "{dim}%h{/}");
        assert_eq!(
            config.display.statusbar.top.foreground_color.resolve(),
            Color::Cyan
        );
        // Unspecified fields use defaults
        assert_eq!(
            config.display.statusbar.top.background_color.resolve(),
            Color::Rgb(30, 30, 30)
        );
        assert_eq!(config.display.statusbar.bottom.status_line, "%b %c");
        assert_eq!(
            config.display.statusbar.bottom.background_color.resolve(),
            Color::Rgb(34, 34, 34)
        );
        assert_eq!(
            config.display.statusbar.bottom.foreground_color.resolve(),
            Color::White
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_statusbar_empty_hides_bar() {
        let dir = std::env::temp_dir().join("git-rt-test-statusbar-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
[display.statusbar.top]
status_line = ""

[display.statusbar.bottom]
status_line = ""
"#,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert!(config.display.statusbar.top.status_line.is_empty());
        assert!(config.display.statusbar.bottom.status_line.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_default_palette_empty() {
        let config = AppConfig::default();
        assert!(config.colors.is_empty());
    }

    #[test]
    fn test_palette_from_toml() {
        let dir = std::env::temp_dir().join("git-rt-test-palette");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r##"
[colors]
danger = "#FF5555"
success = "#50FA7B"
muted = "gray"
"##,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(config.colors.len(), 3);
        assert_eq!(
            config.colors.get("danger").unwrap().resolve(),
            Color::Rgb(255, 85, 85)
        );
        assert_eq!(
            config.colors.get("success").unwrap().resolve(),
            Color::Rgb(80, 250, 123)
        );
        assert_eq!(config.colors.get("muted").unwrap().resolve(), Color::Gray);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_palette_override_builtin() {
        let dir = std::env::temp_dir().join("git-rt-test-palette-override");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r##"
[colors]
red = "#FF6666"
"##,
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert_eq!(
            config.colors.get("red").unwrap().resolve(),
            Color::Rgb(255, 102, 102)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_no_palette_section_defaults_empty() {
        let dir = std::env::temp_dir().join("git-rt-test-no-palette");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "debounce_ms = 100\n").unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();
        assert!(config.colors.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_default_statusbar_config() {
        let config = DisplayConfig::default();
        assert!(config.statusbar.top.status_line.is_empty());
        assert_eq!(
            config.statusbar.top.foreground_color.resolve(),
            Color::White
        );
        assert_eq!(
            config.statusbar.top.background_color.resolve(),
            Color::Rgb(30, 30, 30)
        );
        assert_eq!(
            config.statusbar.bottom.status_line,
            "%b  %c files  {red}%-{/} {green}%+{/}  %=%R"
        );
        assert_eq!(
            config.statusbar.bottom.foreground_color.resolve(),
            Color::White
        );
        assert_eq!(
            config.statusbar.bottom.background_color.resolve(),
            Color::Rgb(30, 30, 30)
        );
    }
}
