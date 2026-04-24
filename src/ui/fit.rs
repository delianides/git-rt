//! Narrow-terminal rendering helpers.
//!
//! Pure, stateless utilities for measuring display width and
//! producing ellipsized strings that fit a target column budget.
//! Uses the `unicode-width` crate so CJK, emoji, and combining
//! marks are measured correctly.

use unicode_width::UnicodeWidthStr;

/// Return the rendered display width of `s` in terminal cells.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_ascii() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn display_width_cjk_is_double() {
        assert_eq!(display_width("日本語"), 6);
    }

    #[test]
    fn display_width_ignores_zero_width() {
        // "e" + combining acute: one column, not two.
        assert_eq!(display_width("e\u{0301}"), 1);
    }
}
