//! Named ANSI colors for the perch TUI.
//!
//! perch renders with ratatui's 16-color ANSI palette so the UI follows the
//! user's terminal theme. These constants are the single source of truth for
//! every color the UI draws; semantic roles map to ANSI names here, and render
//! code refers to the roles rather than picking colors inline.

use ratatui::style::{Color, Style};

// ── Foreground roles ──────────────────────────────────────────────────────
/// Added/insertion counts and "added"/"staged" status.
pub const INSERTIONS: Color = Color::Green;
/// Removed/deletion counts and "deleted" status.
pub const DELETIONS: Color = Color::Red;
/// File paths — inherit the terminal's default foreground.
pub const FILE_PATH: Color = Color::Reset;
/// Header text (repo/branch and segment labels).
pub const HEADER_TEXT: Color = Color::Magenta;
/// Separators between header segments.
pub const HEADER_SEPARATOR: Color = Color::DarkGray;
/// Default (unfocused) pane border.
pub const BORDER: Color = Color::DarkGray;
/// Focused pane border.
pub const BORDER_FOCUSED: Color = Color::Cyan;
/// "Nothing to show" empty-state text.
pub const EMPTY_TEXT: Color = Color::DarkGray;

// ── File status ───────────────────────────────────────────────────────────
pub const STATUS_MODIFIED: Color = Color::Yellow;
pub const STATUS_ADDED: Color = Color::Green;
pub const STATUS_DELETED: Color = Color::Red;
pub const STATUS_RENAMED: Color = Color::Cyan;
/// Untracked files are de-emphasized.
pub const STATUS_UNTRACKED: Color = Color::DarkGray;
/// Staged shares green with added — both are "content entering the tree".
pub const STATUS_STAGED: Color = Color::Green;
/// LightRed (vs plain red for deletions) makes conflicts stand out as urgent.
pub const STATUS_CONFLICTED: Color = Color::LightRed;

// ── Expanded-view section headers ─────────────────────────────────────────
pub const SECTION_CHANGES: Color = Color::Yellow;
pub const SECTION_NEW: Color = Color::Green;
pub const SECTION_COMMITTED: Color = Color::DarkGray;

// ── Diff overlay ──────────────────────────────────────────────────────────
// Diff add/del lines are colored by foreground only; ANSI terminals don't get
// tinted line backgrounds (bg stays the terminal default).
pub const DIFF_ADD_FG: Color = Color::Green;
pub const DIFF_DEL_FG: Color = Color::Red;
pub const DIFF_CONTEXT: Color = Color::Reset;
pub const DIFF_HUNK_HEADER: Color = Color::Cyan;
pub const DIFF_LINE_NUMBER: Color = Color::DarkGray;
pub const DIFF_BORDER: Color = Color::DarkGray;

// ── Highlights ────────────────────────────────────────────────────────────
/// Background of a list item highlighted in the worktree-switch dialog.
pub const FLASH_BG: Color = Color::DarkGray;
/// Background of a file row flashed after a change that net-added lines (or
/// left the line count unchanged).
pub const FLASH_ADD_BG: Color = Color::Green;
/// Background of a file row flashed after a change that net-removed lines.
pub const FLASH_DEL_BG: Color = Color::Red;
/// Border foreground while the pane border is flashing on a change.
pub const FLASH_BORDER: Color = Color::Yellow;

/// Style applied to the selected row: a gray background bar with a bright
/// foreground. The foreground override makes the row legible even where the
/// underlying text is itself gray (e.g. the Committed header, untracked files);
/// the trade-off is that per-span colors (+/- counts, status chars) go
/// monochrome on the selected row only.
pub const SELECTION: Style = Style::new().bg(Color::DarkGray).fg(Color::White);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_locked() {
        assert_eq!(INSERTIONS, Color::Green);
        assert_eq!(DELETIONS, Color::Red);
        assert_eq!(STATUS_MODIFIED, Color::Yellow);
        assert_eq!(STATUS_CONFLICTED, Color::LightRed);
        assert_eq!(FILE_PATH, Color::Reset);
        assert_eq!(BORDER, Color::DarkGray);
        assert_eq!(BORDER_FOCUSED, Color::Cyan);
    }

    #[test]
    fn selection_has_background_bar() {
        assert_eq!(SELECTION.bg, Some(Color::DarkGray));
        assert_eq!(SELECTION.fg, Some(Color::White));
    }
}
