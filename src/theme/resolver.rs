//! Resolve a `ThemeFile` (possibly with `extends`) into a fully-specified `Theme`.
//!
//! The resolver walks the inheritance chain, merging each parent's colors
//! with the child's overrides. If a theme has no `extends` and is not the
//! root `catppuccin-mocha`, missing fields are filled from catppuccin-mocha.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};

use super::color::parse_color;
use super::parser::{ThemeColors, ThemeFile};
use super::Theme;

/// The name of the root theme. Must define every field.
const ROOT_THEME_NAME: &str = "catppuccin-mocha";
/// Maximum allowed depth of the extends chain.
const MAX_EXTENDS_DEPTH: usize = 16;

/// Resolve a `ThemeFile` into a fully-specified `Theme`.
pub fn resolve(file: &ThemeFile, registry: &HashMap<String, ThemeFile>) -> Result<Theme> {
    let mut visited: Vec<String> = Vec::new();
    let merged = merge_chain(file, registry, &mut visited, 0)?;

    // If this theme is not the root, overlay any still-missing fields from the root.
    let final_colors = if file.name != ROOT_THEME_NAME {
        let root = registry
            .get(ROOT_THEME_NAME)
            .ok_or_else(|| anyhow!("root theme '{ROOT_THEME_NAME}' not found in registry"))?;
        let mut root_visited = Vec::new();
        let root_merged = merge_chain(root, registry, &mut root_visited, 0)?;
        fill_missing(&merged, &root_merged)
    } else {
        merged
    };

    build_theme(&file.name, &final_colors)
}

fn merge_chain(
    file: &ThemeFile,
    registry: &HashMap<String, ThemeFile>,
    visited: &mut Vec<String>,
    depth: usize,
) -> Result<ThemeColors> {
    if depth >= MAX_EXTENDS_DEPTH {
        return Err(anyhow!(
            "extends chain too deep (> {MAX_EXTENDS_DEPTH}) starting at '{}'",
            file.name
        ));
    }
    if visited.iter().any(|v| v == &file.name) {
        return Err(anyhow!(
            "circular extends chain detected at '{}'",
            file.name
        ));
    }
    visited.push(file.name.clone());

    let parent_colors = if let Some(parent_name) = &file.extends {
        let parent = registry.get(parent_name).ok_or_else(|| {
            anyhow!(
                "theme '{}' extends unknown theme '{}'",
                file.name,
                parent_name
            )
        })?;
        merge_chain(parent, registry, visited, depth + 1)?
    } else {
        ThemeColors::default()
    };

    Ok(overlay(parent_colors, &file.colors))
}

fn overlay(parent: ThemeColors, child: &ThemeColors) -> ThemeColors {
    ThemeColors {
        bg: child.bg.clone().or(parent.bg),
        fg: child.fg.clone().or(parent.fg),
        border: child.border.clone().or(parent.border),
        border_focused: child.border_focused.clone().or(parent.border_focused),
        header_text: child.header_text.clone().or(parent.header_text),
        header_separator: child.header_separator.clone().or(parent.header_separator),
        file_path: child.file_path.clone().or(parent.file_path),
        file_insertions: child.file_insertions.clone().or(parent.file_insertions),
        file_deletions: child.file_deletions.clone().or(parent.file_deletions),
        selection_bg: child.selection_bg.clone().or(parent.selection_bg),
        selection_fg: child.selection_fg.clone().or(parent.selection_fg),
        flash_bg: child.flash_bg.clone().or(parent.flash_bg),
        empty_text: child.empty_text.clone().or(parent.empty_text),
        diff_add_fg: child.diff_add_fg.clone().or(parent.diff_add_fg),
        diff_add_bg: child.diff_add_bg.clone().or(parent.diff_add_bg),
        diff_del_fg: child.diff_del_fg.clone().or(parent.diff_del_fg),
        diff_del_bg: child.diff_del_bg.clone().or(parent.diff_del_bg),
        diff_context: child.diff_context.clone().or(parent.diff_context),
        diff_hunk_header: child.diff_hunk_header.clone().or(parent.diff_hunk_header),
        diff_line_number: child.diff_line_number.clone().or(parent.diff_line_number),
        diff_border: child.diff_border.clone().or(parent.diff_border),
        status_modified: child.status_modified.clone().or(parent.status_modified),
        status_added: child.status_added.clone().or(parent.status_added),
        status_deleted: child.status_deleted.clone().or(parent.status_deleted),
        status_renamed: child.status_renamed.clone().or(parent.status_renamed),
        status_untracked: child.status_untracked.clone().or(parent.status_untracked),
        status_staged: child.status_staged.clone().or(parent.status_staged),
        status_conflicted: child.status_conflicted.clone().or(parent.status_conflicted),
        section_changes: child.section_changes.clone().or(parent.section_changes),
        section_new: child.section_new.clone().or(parent.section_new),
        section_committed: child.section_committed.clone().or(parent.section_committed),
    }
}

fn fill_missing(child: &ThemeColors, fallback: &ThemeColors) -> ThemeColors {
    overlay(fallback.clone(), child)
}

fn build_theme(name: &str, colors: &ThemeColors) -> Result<Theme> {
    macro_rules! parse_field {
        ($field:ident) => {
            parse_color(colors.$field.as_deref().ok_or_else(|| {
                anyhow!("theme '{}' missing field '{}'", name, stringify!($field))
            })?)
            .with_context(|| format!("theme '{}' field '{}'", name, stringify!($field)))?
        };
    }

    macro_rules! parse_field_or_default {
        ($field:ident, $default:expr) => {
            if let Some(ref val) = colors.$field {
                parse_color(val)
                    .with_context(|| format!("theme '{}' field '{}'", name, stringify!($field)))?
            } else {
                parse_color($default).expect("hardcoded default must be valid")
            }
        };
    }

    Ok(Theme {
        name: name.to_string(),
        bg: parse_field!(bg),
        fg: parse_field!(fg),
        border: parse_field!(border),
        border_focused: parse_field!(border_focused),
        header_text: parse_field!(header_text),
        header_separator: parse_field!(header_separator),
        file_path: parse_field!(file_path),
        file_insertions: parse_field!(file_insertions),
        file_deletions: parse_field!(file_deletions),
        selection_bg: parse_field!(selection_bg),
        selection_fg: parse_field!(selection_fg),
        flash_bg: parse_field!(flash_bg),
        empty_text: parse_field!(empty_text),
        diff_add_fg: parse_field!(diff_add_fg),
        diff_add_bg: parse_field!(diff_add_bg),
        diff_del_fg: parse_field!(diff_del_fg),
        diff_del_bg: parse_field!(diff_del_bg),
        diff_context: parse_field!(diff_context),
        diff_hunk_header: parse_field!(diff_hunk_header),
        diff_line_number: parse_field!(diff_line_number),
        diff_border: parse_field!(diff_border),
        status_modified: parse_field_or_default!(status_modified, "#e5c07b"),
        status_added: parse_field_or_default!(status_added, "#98c379"),
        status_deleted: parse_field_or_default!(status_deleted, "#e06c75"),
        status_renamed: parse_field_or_default!(status_renamed, "#56b6c2"),
        status_untracked: parse_field_or_default!(status_untracked, "#7f848e"),
        status_staged: parse_field_or_default!(status_staged, "#98c379"),
        status_conflicted: parse_field_or_default!(status_conflicted, "#be5046"),
        section_changes: parse_field_or_default!(section_changes, "#e5c07b"),
        section_new: parse_field_or_default!(section_new, "#98c379"),
        section_committed: parse_field_or_default!(section_committed, "#7f848e"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn make_complete_mocha() -> ThemeFile {
        ThemeFile {
            name: ROOT_THEME_NAME.to_string(),
            extends: None,
            colors: ThemeColors {
                bg: Some("#000000".into()),
                fg: Some("#ffffff".into()),
                border: Some("#111111".into()),
                border_focused: Some("#222222".into()),
                header_text: Some("#333333".into()),
                header_separator: Some("#444444".into()),
                file_path: Some("#555555".into()),
                file_insertions: Some("#00ff00".into()),
                file_deletions: Some("#ff0000".into()),
                selection_bg: Some("#666666".into()),
                selection_fg: Some("#777777".into()),
                flash_bg: Some("#888888".into()),
                empty_text: Some("#999999".into()),
                diff_add_fg: Some("#aaaaaa".into()),
                diff_add_bg: Some("#bbbbbb".into()),
                diff_del_fg: Some("#cccccc".into()),
                diff_del_bg: Some("#dddddd".into()),
                diff_context: Some("#eeeeee".into()),
                diff_hunk_header: Some("#123456".into()),
                diff_line_number: Some("#234567".into()),
                diff_border: Some("#345678".into()),
                status_modified: None,
                status_added: None,
                status_deleted: None,
                status_renamed: None,
                status_untracked: None,
                status_staged: None,
                status_conflicted: None,
                section_changes: None,
                section_new: None,
                section_committed: None,
            },
        }
    }

    fn registry_with_mocha() -> HashMap<String, ThemeFile> {
        let mut r = HashMap::new();
        r.insert(ROOT_THEME_NAME.to_string(), make_complete_mocha());
        r
    }

    #[test]
    fn test_resolve_root_theme() {
        let mocha = make_complete_mocha();
        let registry = registry_with_mocha();
        let theme = resolve(&mocha, &registry).unwrap();
        assert_eq!(theme.name, "catppuccin-mocha");
        assert_eq!(theme.bg, Color::Rgb(0, 0, 0));
        assert_eq!(theme.fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_resolve_with_extends_fills_missing() {
        let child = ThemeFile {
            name: "my-theme".to_string(),
            extends: Some(ROOT_THEME_NAME.to_string()),
            colors: ThemeColors {
                bg: Some("#abcdef".into()),
                ..Default::default()
            },
        };
        let registry = registry_with_mocha();
        let theme = resolve(&child, &registry).unwrap();
        assert_eq!(theme.bg, Color::Rgb(0xab, 0xcd, 0xef));
        assert_eq!(theme.fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_resolve_implicit_fallback_to_root() {
        let child = ThemeFile {
            name: "sparse".to_string(),
            extends: None,
            colors: ThemeColors {
                bg: Some("#abcdef".into()),
                ..Default::default()
            },
        };
        let registry = registry_with_mocha();
        let theme = resolve(&child, &registry).unwrap();
        assert_eq!(theme.bg, Color::Rgb(0xab, 0xcd, 0xef));
        assert_eq!(theme.fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_resolve_extends_unknown_parent_fails() {
        let child = ThemeFile {
            name: "orphan".to_string(),
            extends: Some("nonexistent".to_string()),
            colors: ThemeColors::default(),
        };
        let registry = registry_with_mocha();
        let err = resolve(&child, &registry).unwrap_err();
        assert!(err.to_string().contains("unknown theme"));
    }

    #[test]
    fn test_resolve_circular_extends_fails() {
        let mut registry = registry_with_mocha();
        registry.insert(
            "a".to_string(),
            ThemeFile {
                name: "a".to_string(),
                extends: Some("b".to_string()),
                colors: ThemeColors::default(),
            },
        );
        registry.insert(
            "b".to_string(),
            ThemeFile {
                name: "b".to_string(),
                extends: Some("a".to_string()),
                colors: ThemeColors::default(),
            },
        );
        let err = resolve(registry.get("a").unwrap(), &registry).unwrap_err();
        assert!(err.to_string().contains("circular"));
    }

    #[test]
    fn test_resolve_missing_root_field_fails() {
        let mut broken_root = make_complete_mocha();
        broken_root.colors.bg = None;
        let mut registry = HashMap::new();
        registry.insert(ROOT_THEME_NAME.to_string(), broken_root);
        let err = resolve(registry.get(ROOT_THEME_NAME).unwrap(), &registry).unwrap_err();
        assert!(err.to_string().contains("missing field"));
    }

    #[test]
    fn test_status_fields_use_hardcoded_fallback() {
        let registry = registry_with_mocha();
        let mocha = registry.get(ROOT_THEME_NAME).unwrap();
        let theme = resolve(mocha, &registry).unwrap();
        assert_eq!(theme.status_modified, Color::Rgb(0xe5, 0xc0, 0x7b));
        assert_eq!(theme.status_added, Color::Rgb(0x98, 0xc3, 0x79));
        assert_eq!(theme.status_deleted, Color::Rgb(0xe0, 0x6c, 0x75));
        assert_eq!(theme.status_renamed, Color::Rgb(0x56, 0xb6, 0xc2));
        assert_eq!(theme.status_untracked, Color::Rgb(0x7f, 0x84, 0x8e));
        assert_eq!(theme.status_staged, Color::Rgb(0x98, 0xc3, 0x79));
        assert_eq!(theme.status_conflicted, Color::Rgb(0xbe, 0x50, 0x46));
    }

    #[test]
    fn test_status_fields_override_fallback() {
        let mut registry = registry_with_mocha();
        registry.insert(
            "custom".to_string(),
            ThemeFile {
                name: "custom".to_string(),
                extends: Some(ROOT_THEME_NAME.to_string()),
                colors: ThemeColors {
                    status_modified: Some("#aabbcc".into()),
                    ..Default::default()
                },
            },
        );
        let theme = resolve(registry.get("custom").unwrap(), &registry).unwrap();
        // Provided value wins
        assert_eq!(theme.status_modified, Color::Rgb(0xaa, 0xbb, 0xcc));
        // Others still fall back
        assert_eq!(theme.status_added, Color::Rgb(0x98, 0xc3, 0x79));
    }

    #[test]
    fn test_resolve_invalid_color_fails() {
        let mut broken = make_complete_mocha();
        broken.colors.bg = Some("not-a-color".to_string());
        let mut registry = HashMap::new();
        registry.insert(ROOT_THEME_NAME.to_string(), broken);
        let err = resolve(registry.get(ROOT_THEME_NAME).unwrap(), &registry).unwrap_err();
        assert!(err.to_string().contains("bg"));
    }

    #[test]
    fn test_resolve_child_overrides_parent() {
        let mut registry = registry_with_mocha();
        registry.insert(
            "child".to_string(),
            ThemeFile {
                name: "child".to_string(),
                extends: Some(ROOT_THEME_NAME.to_string()),
                colors: ThemeColors {
                    fg: Some("#ff00ff".into()),
                    ..Default::default()
                },
            },
        );
        let theme = resolve(registry.get("child").unwrap(), &registry).unwrap();
        assert_eq!(theme.fg, Color::Rgb(0xff, 0x00, 0xff));
        assert_eq!(theme.bg, Color::Rgb(0, 0, 0));
    }
}
