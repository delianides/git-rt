//! Theme system for git-rt.
//!
//! Provides a set of built-in colour themes that can be selected by name.
//! All theme data is static — there are no user-defined themes, only theme
//! selection via configuration.

pub mod catalog;

use ratatui::style::Color;

pub use catalog::ALL_THEMES;

/// A complete colour theme for the git-rt TUI.
///
/// Every field is a [`Color`] value from ratatui so it can be used directly
/// in [`Style`](ratatui::style::Style) calls without conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Human-readable identifier (e.g. `"catppuccin-mocha"`).
    pub name: &'static str,

    // ── Pane borders ──────────────────────────────────────────────────────
    /// Default (unfocused) border colour.
    pub border: Color,
    /// Focused pane border colour.
    pub border_focused: Color,

    // ── Header bar ────────────────────────────────────────────────────────
    /// Header text / title colour.
    pub header_text: Color,
    /// Separator line in the header.
    pub header_separator: Color,

    // ── File list ─────────────────────────────────────────────────────────
    /// File path text colour.
    pub file_path: Color,
    /// Insertion count (`+N`) colour.
    pub file_insertions: Color,
    /// Deletion count (`-N`) colour.
    pub file_deletions: Color,

    // ── UI state ──────────────────────────────────────────────────────────
    /// Background of the selected row.
    pub selection_bg: Color,
    /// Foreground of the selected row.
    pub selection_fg: Color,
    /// Background used for brief "flash" feedback.
    pub flash_bg: Color,
    /// Colour for "nothing to show" placeholder text.
    pub empty_text: Color,

    // ── Diff rendering ────────────────────────────────────────────────────
    /// Foreground for added lines (`+`).
    pub diff_add_fg: Color,
    /// Background for added lines.
    pub diff_add_bg: Color,
    /// Foreground for removed lines (`-`).
    pub diff_del_fg: Color,
    /// Background for removed lines.
    pub diff_del_bg: Color,
    /// Colour for context (unchanged) diff lines.
    pub diff_context: Color,
    /// Colour for hunk headers (`@@ … @@`).
    pub diff_hunk_header: Color,
    /// Colour for line numbers in the diff gutter.
    pub diff_line_number: Color,
    /// Border around the inline diff panel.
    pub diff_border: Color,

    // ── Base ──────────────────────────────────────────────────────────────
    /// Terminal background colour.
    pub bg: Color,
    /// Default foreground / text colour.
    pub fg: Color,
}

/// Return the theme matching `name` (case-insensitive).
///
/// Falls back to the first theme in [`ALL_THEMES`] when no match is found.
pub fn get_theme(name: &str) -> &'static Theme {
    let lower = name.to_lowercase();
    ALL_THEMES
        .iter()
        .find(|t| t.name.to_lowercase() == lower)
        .unwrap_or(&ALL_THEMES[0])
}

/// Return the names of all available themes.
pub fn list_themes() -> Vec<&'static str> {
    ALL_THEMES.iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_default_theme() {
        let theme = get_theme("catppuccin-mocha");
        assert_eq!(theme.name, "catppuccin-mocha");
    }

    #[test]
    fn test_get_theme_case_insensitive() {
        let theme = get_theme("Catppuccin-Mocha");
        assert_eq!(theme.name, "catppuccin-mocha");
    }

    #[test]
    fn test_get_theme_fallback() {
        let theme = get_theme("this-theme-does-not-exist");
        // Should fall back to the first theme (catppuccin-mocha)
        assert_eq!(theme.name, ALL_THEMES[0].name);
    }

    #[test]
    fn test_list_themes_not_empty() {
        let themes = list_themes();
        assert!(
            themes.len() >= 5,
            "expected at least 5 themes, got {}",
            themes.len()
        );
    }

    #[test]
    fn test_all_themes_have_distinct_names() {
        let mut names: Vec<&str> = ALL_THEMES.iter().map(|t| t.name).collect();
        let original_len = names.len();
        names.dedup();
        // Also sort+dedup to catch non-adjacent duplicates
        let mut sorted = ALL_THEMES.iter().map(|t| t.name).collect::<Vec<_>>();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), original_len, "duplicate theme names detected");
    }
}
