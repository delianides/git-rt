//! Theme system for git-rt.
//!
//! Themes are parsed from TOML or JSON files. Built-in themes are embedded
//! in the binary via `include_str!`. Users can place custom themes in
//! `~/.config/git-rt/themes/` (or the platform equivalent) to override
//! built-ins or add new themes. Themes may use `extends = "<name>"` to
//! inherit missing fields from another theme.

pub mod color;
pub mod parser;
pub mod resolver;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ratatui::style::Color;

use parser::ThemeFile;

/// A complete, fully-resolved colour theme for the git-rt TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: String,

    pub border: Color,
    pub border_focused: Color,

    pub header_text: Color,
    pub header_separator: Color,

    pub file_path: Color,
    pub file_insertions: Color,
    pub file_deletions: Color,

    pub selection_bg: Color,
    pub selection_fg: Color,
    pub flash_bg: Color,
    pub empty_text: Color,

    pub diff_add_fg: Color,
    pub diff_add_bg: Color,
    pub diff_del_fg: Color,
    pub diff_del_bg: Color,
    pub diff_context: Color,
    pub diff_hunk_header: Color,
    pub diff_line_number: Color,
    pub diff_border: Color,

    pub bg: Color,
    pub fg: Color,
}

/// Name of the default theme used when resolution fails or none is specified.
pub const DEFAULT_THEME_NAME: &str = "catppuccin-mocha";

/// Built-in theme TOML sources, embedded at compile time.
const BUILTIN_THEMES: &[(&str, &str)] = &[
    (
        "catppuccin-mocha",
        include_str!("builtin/catppuccin-mocha.toml"),
    ),
    (
        "catppuccin-latte",
        include_str!("builtin/catppuccin-latte.toml"),
    ),
    ("one-dark", include_str!("builtin/one-dark.toml")),
    ("dracula", include_str!("builtin/dracula.toml")),
    ("gruvbox-dark", include_str!("builtin/gruvbox-dark.toml")),
    ("nord", include_str!("builtin/nord.toml")),
    ("tokyo-night", include_str!("builtin/tokyo-night.toml")),
    (
        "solarized-dark",
        include_str!("builtin/solarized-dark.toml"),
    ),
    ("rose-pine", include_str!("builtin/rose-pine.toml")),
    ("kanagawa", include_str!("builtin/kanagawa.toml")),
    (
        "everforest-dark",
        include_str!("builtin/everforest-dark.toml"),
    ),
];

/// Default path to the user themes directory.
/// Follows the same resolution logic as the config file: `~/.config/git-rt/themes/`
/// or the platform-specific config directory fallback.
pub fn default_user_themes_dir() -> Option<PathBuf> {
    let xdg = dirs::home_dir().map(|h| h.join(".config").join("git-rt").join("themes"));
    if let Some(ref p) = xdg {
        if p.exists() {
            return xdg;
        }
    }
    dirs::config_dir().map(|d| d.join("git-rt").join("themes"))
}

/// Build the theme registry by parsing all built-in themes and any user themes
/// in the given directory. User themes override built-ins with the same name.
pub fn build_registry(user_themes_dir: Option<&Path>) -> HashMap<String, ThemeFile> {
    let mut registry: HashMap<String, ThemeFile> = HashMap::new();

    // Load built-ins
    for (name, contents) in BUILTIN_THEMES {
        match parser::parse_toml(contents) {
            Ok(file) => {
                registry.insert(file.name.clone(), file);
            }
            Err(e) => {
                tracing::warn!(theme = name, error = %e, "failed to parse built-in theme");
            }
        }
    }

    // Load user themes (TOML and JSON)
    if let Some(dir) = user_themes_dir {
        if dir.is_dir() {
            match std::fs::read_dir(dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let ext = path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(str::to_lowercase);
                        if !matches!(ext.as_deref(), Some("toml") | Some("json")) {
                            continue;
                        }
                        match parser::parse_file(&path) {
                            Ok(file) => {
                                tracing::debug!(
                                    name = %file.name,
                                    path = %path.display(),
                                    "loaded user theme"
                                );
                                registry.insert(file.name.clone(), file);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    path = %path.display(),
                                    error = %e,
                                    "failed to parse user theme"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(dir = %dir.display(), error = %e, "failed to read user themes dir");
                }
            }
        }
    }

    registry
}

/// Determine whether `name_or_path` looks like a filesystem path.
fn looks_like_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.ends_with(".toml") || s.ends_with(".json")
}

/// Load and resolve a theme by name or file path.
///
/// Resolution order:
/// 1. If `name_or_path` looks like a path, load the file directly
/// 2. Otherwise, look up the name in the registry
/// 3. Resolve the `extends` chain, filling any missing fields from the root theme
///
/// On any error, logs a warning and falls back to the default theme.
pub fn load_theme(name_or_path: &str, user_themes_dir: Option<&Path>) -> Theme {
    match try_load_theme(name_or_path, user_themes_dir) {
        Ok(theme) => theme,
        Err(e) => {
            tracing::warn!(
                theme = name_or_path,
                error = %e,
                "failed to load theme, falling back to default"
            );
            let registry = build_registry(user_themes_dir);
            let default_file = registry
                .get(DEFAULT_THEME_NAME)
                .cloned()
                .expect("built-in default theme must always exist in registry");
            resolver::resolve(&default_file, &registry)
                .expect("built-in default theme must always resolve")
        }
    }
}

/// Fallible version of `load_theme`. Returns an error instead of falling back.
pub fn try_load_theme(name_or_path: &str, user_themes_dir: Option<&Path>) -> Result<Theme> {
    let registry = build_registry(user_themes_dir);

    let file = if looks_like_path(name_or_path) {
        parser::parse_file(Path::new(name_or_path))
            .with_context(|| format!("failed to load theme file: {name_or_path}"))?
    } else {
        registry
            .get(name_or_path)
            .cloned()
            .ok_or_else(|| anyhow!("unknown theme: '{name_or_path}'"))?
    };

    resolver::resolve(&file, &registry)
}

/// Return the names of all available themes (built-in + user).
pub fn list_themes(user_themes_dir: Option<&Path>) -> Vec<String> {
    let registry = build_registry(user_themes_dir);
    let mut names: Vec<String> = registry.keys().cloned().collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_theme() {
        let theme = load_theme(DEFAULT_THEME_NAME, None);
        assert_eq!(theme.name, DEFAULT_THEME_NAME);
    }

    #[test]
    fn test_load_known_builtin_theme() {
        let theme = load_theme("dracula", None);
        assert_eq!(theme.name, "dracula");
    }

    #[test]
    fn test_all_builtin_themes_resolve() {
        let builtins = [
            "catppuccin-mocha",
            "catppuccin-latte",
            "one-dark",
            "dracula",
            "gruvbox-dark",
            "nord",
            "tokyo-night",
            "solarized-dark",
            "rose-pine",
            "kanagawa",
            "everforest-dark",
        ];
        for name in builtins {
            let theme = load_theme(name, None);
            assert_eq!(theme.name, name, "theme {name} did not load correctly");
        }
    }

    #[test]
    fn test_load_unknown_theme_falls_back() {
        let theme = load_theme("this-does-not-exist", None);
        assert_eq!(theme.name, DEFAULT_THEME_NAME);
    }

    #[test]
    fn test_list_themes_includes_builtins() {
        let themes = list_themes(None);
        assert!(themes.contains(&"catppuccin-mocha".to_string()));
        assert!(themes.contains(&"dracula".to_string()));
        assert!(themes.len() >= 11);
    }

    #[test]
    fn test_looks_like_path() {
        assert!(looks_like_path("/absolute/path.toml"));
        assert!(looks_like_path("relative/path.toml"));
        assert!(looks_like_path("file.toml"));
        assert!(looks_like_path("file.json"));
        assert!(!looks_like_path("catppuccin-mocha"));
        assert!(!looks_like_path("my-theme"));
    }

    #[test]
    fn test_load_from_file_path() {
        let dir = std::env::temp_dir().join("git-rt-theme-file-load");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.toml");
        std::fs::write(
            &path,
            r##"
name = "file-theme"
extends = "catppuccin-mocha"

[colors]
bg = "#123456"
"##,
        )
        .unwrap();

        let theme = load_theme(path.to_str().unwrap(), None);
        assert_eq!(theme.name, "file-theme");
        assert_eq!(theme.bg, Color::Rgb(0x12, 0x34, 0x56));
        // fg inherited from catppuccin-mocha
        assert_eq!(theme.fg, Color::Rgb(0xcd, 0xd6, 0xf4));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_user_themes_override_builtins() {
        let dir = std::env::temp_dir().join("git-rt-theme-override");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dracula.toml");
        std::fs::write(
            &path,
            r##"
name = "dracula"
extends = "catppuccin-mocha"

[colors]
bg = "#aabbcc"
"##,
        )
        .unwrap();

        let theme = load_theme("dracula", Some(&dir));
        assert_eq!(theme.name, "dracula");
        assert_eq!(theme.bg, Color::Rgb(0xaa, 0xbb, 0xcc));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_user_themes_json_format() {
        let dir = std::env::temp_dir().join("git-rt-theme-json");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("my-json-theme.json");
        std::fs::write(
            &path,
            r##"{
                "name": "my-json-theme",
                "extends": "catppuccin-mocha",
                "colors": {
                    "bg": "#ff0000"
                }
            }"##,
        )
        .unwrap();

        let theme = load_theme("my-json-theme", Some(&dir));
        assert_eq!(theme.name, "my-json-theme");
        assert_eq!(theme.bg, Color::Rgb(0xff, 0, 0));

        std::fs::remove_dir_all(&dir).ok();
    }
}
