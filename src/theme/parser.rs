//! Parse theme definition files (TOML and JSON) into a raw `ThemeFile`.
//!
//! `ThemeFile` holds optional fields so inheritance (`extends`) can fill in
//! missing values later. The resolver in `resolver.rs` handles that.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// A raw theme file as parsed from disk or an embedded string.
/// All color fields are optional — missing fields inherit from the parent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThemeFile {
    /// The theme's canonical name (required).
    pub name: String,
    /// Optional parent theme to inherit from.
    #[serde(default)]
    pub extends: Option<String>,
    /// Color values (all optional).
    #[serde(default)]
    pub colors: ThemeColors,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThemeColors {
    pub bg: Option<String>,
    pub fg: Option<String>,
    pub border: Option<String>,
    pub border_focused: Option<String>,
    pub header_text: Option<String>,
    pub header_separator: Option<String>,
    pub file_path: Option<String>,
    pub file_insertions: Option<String>,
    pub file_deletions: Option<String>,
    pub selection_bg: Option<String>,
    pub selection_fg: Option<String>,
    pub flash_bg: Option<String>,
    pub empty_text: Option<String>,
    pub diff_add_fg: Option<String>,
    pub diff_add_bg: Option<String>,
    pub diff_del_fg: Option<String>,
    pub diff_del_bg: Option<String>,
    pub diff_context: Option<String>,
    pub diff_hunk_header: Option<String>,
    pub diff_line_number: Option<String>,
    pub diff_border: Option<String>,
    pub status_modified: Option<String>,
    pub status_added: Option<String>,
    pub status_deleted: Option<String>,
    pub status_renamed: Option<String>,
    pub status_untracked: Option<String>,
    pub status_staged: Option<String>,
    pub status_conflicted: Option<String>,
}

/// Parse a TOML theme string.
pub fn parse_toml(input: &str) -> Result<ThemeFile> {
    toml::from_str::<ThemeFile>(input).context("failed to parse TOML theme file")
}

/// Parse a JSON theme string.
pub fn parse_json(input: &str) -> Result<ThemeFile> {
    serde_json::from_str::<ThemeFile>(input).context("failed to parse JSON theme file")
}

/// Parse a theme from a file path. Format is chosen by extension (`.toml` or `.json`).
pub fn parse_file(path: &Path) -> Result<ThemeFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read theme file: {}", path.display()))?;

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase);

    match ext.as_deref() {
        Some("json") => parse_json(&contents),
        _ => parse_toml(&contents), // default to TOML
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_toml_minimal() {
        let input = r#"
name = "test"
"#;
        let theme = parse_toml(input).unwrap();
        assert_eq!(theme.name, "test");
        assert!(theme.extends.is_none());
        assert!(theme.colors.bg.is_none());
    }

    #[test]
    fn test_parse_toml_with_extends() {
        let input = r#"
name = "my-theme"
extends = "catppuccin-mocha"
"#;
        let theme = parse_toml(input).unwrap();
        assert_eq!(theme.name, "my-theme");
        assert_eq!(theme.extends.as_deref(), Some("catppuccin-mocha"));
    }

    #[test]
    fn test_parse_toml_with_colors() {
        let input = r##"
name = "test"

[colors]
bg = "#1e1e2e"
fg = "#cdd6f4"
diff_add_fg = "green"
"##;
        let theme = parse_toml(input).unwrap();
        assert_eq!(theme.colors.bg.as_deref(), Some("#1e1e2e"));
        assert_eq!(theme.colors.fg.as_deref(), Some("#cdd6f4"));
        assert_eq!(theme.colors.diff_add_fg.as_deref(), Some("green"));
        assert!(theme.colors.border.is_none());
    }

    #[test]
    fn test_parse_toml_invalid() {
        let input = "not valid toml ][";
        assert!(parse_toml(input).is_err());
    }

    #[test]
    fn test_parse_toml_missing_name() {
        // `name` is required — serde should fail without it
        let input = r##"
[colors]
bg = "#000000"
"##;
        assert!(parse_toml(input).is_err());
    }

    #[test]
    fn test_parse_json_minimal() {
        let input = r#"{"name": "test"}"#;
        let theme = parse_json(input).unwrap();
        assert_eq!(theme.name, "test");
    }

    #[test]
    fn test_parse_json_with_colors() {
        let input = r##"{
            "name": "test",
            "extends": "dracula",
            "colors": {
                "bg": "#000000",
                "fg": "#ffffff"
            }
        }"##;
        let theme = parse_json(input).unwrap();
        assert_eq!(theme.name, "test");
        assert_eq!(theme.extends.as_deref(), Some("dracula"));
        assert_eq!(theme.colors.bg.as_deref(), Some("#000000"));
        assert_eq!(theme.colors.fg.as_deref(), Some("#ffffff"));
    }

    #[test]
    fn test_builtin_themes_parse() {
        let files = [
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

        for (name, content) in files {
            let parsed =
                parse_toml(content).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"));
            assert_eq!(parsed.name, name, "name mismatch for {name}");
            assert!(parsed.colors.bg.is_some(), "{name}: missing bg");
            assert!(parsed.colors.fg.is_some(), "{name}: missing fg");
            assert!(parsed.colors.border.is_some(), "{name}: missing border");
            assert!(
                parsed.colors.border_focused.is_some(),
                "{name}: missing border_focused"
            );
            assert!(
                parsed.colors.header_text.is_some(),
                "{name}: missing header_text"
            );
            assert!(
                parsed.colors.header_separator.is_some(),
                "{name}: missing header_separator"
            );
            assert!(
                parsed.colors.file_path.is_some(),
                "{name}: missing file_path"
            );
            assert!(
                parsed.colors.file_insertions.is_some(),
                "{name}: missing file_insertions"
            );
            assert!(
                parsed.colors.file_deletions.is_some(),
                "{name}: missing file_deletions"
            );
            assert!(
                parsed.colors.selection_bg.is_some(),
                "{name}: missing selection_bg"
            );
            assert!(
                parsed.colors.selection_fg.is_some(),
                "{name}: missing selection_fg"
            );
            assert!(parsed.colors.flash_bg.is_some(), "{name}: missing flash_bg");
            assert!(
                parsed.colors.empty_text.is_some(),
                "{name}: missing empty_text"
            );
            assert!(
                parsed.colors.diff_add_fg.is_some(),
                "{name}: missing diff_add_fg"
            );
            assert!(
                parsed.colors.diff_add_bg.is_some(),
                "{name}: missing diff_add_bg"
            );
            assert!(
                parsed.colors.diff_del_fg.is_some(),
                "{name}: missing diff_del_fg"
            );
            assert!(
                parsed.colors.diff_del_bg.is_some(),
                "{name}: missing diff_del_bg"
            );
            assert!(
                parsed.colors.diff_context.is_some(),
                "{name}: missing diff_context"
            );
            assert!(
                parsed.colors.diff_hunk_header.is_some(),
                "{name}: missing diff_hunk_header"
            );
            assert!(
                parsed.colors.diff_line_number.is_some(),
                "{name}: missing diff_line_number"
            );
            assert!(
                parsed.colors.diff_border.is_some(),
                "{name}: missing diff_border"
            );
            assert!(
                parsed.colors.status_modified.is_some(),
                "{name}: missing status_modified"
            );
            assert!(
                parsed.colors.status_added.is_some(),
                "{name}: missing status_added"
            );
            assert!(
                parsed.colors.status_deleted.is_some(),
                "{name}: missing status_deleted"
            );
            assert!(
                parsed.colors.status_renamed.is_some(),
                "{name}: missing status_renamed"
            );
            assert!(
                parsed.colors.status_untracked.is_some(),
                "{name}: missing status_untracked"
            );
            assert!(
                parsed.colors.status_staged.is_some(),
                "{name}: missing status_staged"
            );
            assert!(
                parsed.colors.status_conflicted.is_some(),
                "{name}: missing status_conflicted"
            );
        }
    }

    #[test]
    fn test_parse_file_dispatches_by_extension() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("git-rt-theme-parse-test");
        std::fs::create_dir_all(&dir).unwrap();

        let toml_path = dir.join("a.toml");
        std::fs::write(&toml_path, r#"name = "toml-theme""#).unwrap();
        let parsed = parse_file(&toml_path).unwrap();
        assert_eq!(parsed.name, "toml-theme");

        let json_path = dir.join("b.json");
        let mut f = std::fs::File::create(&json_path).unwrap();
        f.write_all(br#"{"name": "json-theme"}"#).unwrap();
        let parsed = parse_file(&json_path).unwrap();
        assert_eq!(parsed.name, "json-theme");

        std::fs::remove_dir_all(&dir).ok();
    }
}
