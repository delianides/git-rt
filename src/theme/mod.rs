//! Theme system for git-rt.
//!
//! Themes are loaded from TOML/JSON files and resolved into fully-specified
//! `Theme` values via the `resolver` module. The `parser` module handles raw
//! file parsing; `color` handles color string parsing.

pub mod color;
pub mod parser;
pub mod resolver;

use ratatui::style::Color;

/// A complete colour theme for the git-rt TUI.
///
/// Every field is a [`Color`] value from ratatui so it can be used directly
/// in [`Style`](ratatui::style::Style) calls without conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Human-readable identifier (e.g. `"catppuccin-mocha"`).
    pub name: String,

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

/// Temporary stub so the binary continues to compile while the theme system
/// is being rewritten. Task 5 replaces this with a real registry + loader.
///
/// Returns a minimal placeholder theme. The name is preserved so call sites
/// that inspect it still work.
#[doc(hidden)]
pub fn get_theme(name: &str) -> Theme {
    Theme {
        name: name.to_string(),
        bg: Color::Reset,
        fg: Color::Reset,
        border: Color::Reset,
        border_focused: Color::Reset,
        header_text: Color::Reset,
        header_separator: Color::Reset,
        file_path: Color::Reset,
        file_insertions: Color::Green,
        file_deletions: Color::Red,
        selection_bg: Color::DarkGray,
        selection_fg: Color::White,
        flash_bg: Color::DarkGray,
        empty_text: Color::Gray,
        diff_add_fg: Color::Green,
        diff_add_bg: Color::Reset,
        diff_del_fg: Color::Red,
        diff_del_bg: Color::Reset,
        diff_context: Color::Gray,
        diff_hunk_header: Color::Cyan,
        diff_line_number: Color::DarkGray,
        diff_border: Color::Reset,
    }
}
