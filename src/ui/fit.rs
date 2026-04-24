//! Narrow-terminal rendering helpers.
//!
//! Pure, stateless utilities for measuring display width and
//! producing ellipsized strings that fit a target column budget.
//! Uses the `unicode-width` crate so CJK, emoji, and combining
//! marks are measured correctly.

use std::borrow::Cow;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ELLIPSIS: &str = "\u{2026}";
const ELLIPSIS_WIDTH: usize = 1;

/// Return the rendered display width of `s` in terminal cells.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// End-ellipsis truncation. Returns `s` unchanged when it fits.
/// Otherwise returns `"<head>…"` with total display width ≤ `max_cols`.
/// Never splits inside a multi-byte char.
pub fn truncate_end(s: &str, max_cols: usize) -> Cow<'_, str> {
    if max_cols == 0 {
        return Cow::Borrowed("");
    }
    if display_width(s) <= max_cols {
        return Cow::Borrowed(s);
    }
    if max_cols <= ELLIPSIS_WIDTH {
        return Cow::Owned(ELLIPSIS.to_string());
    }

    let budget = max_cols - ELLIPSIS_WIDTH;
    let mut out = String::with_capacity(s.len());
    let mut used = 0usize;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(ELLIPSIS);
    Cow::Owned(out)
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

    #[test]
    fn truncate_end_already_fits_returns_unchanged() {
        assert_eq!(truncate_end("hi", 5), "hi");
    }

    #[test]
    fn truncate_end_exact_fit_returns_unchanged() {
        assert_eq!(truncate_end("hello", 5), "hello");
    }

    #[test]
    fn truncate_end_produces_ellipsis_suffix() {
        assert_eq!(truncate_end("hello world", 6), "hello\u{2026}");
    }

    #[test]
    fn truncate_end_respects_cjk_width() {
        // "日本語" is 6 cols; at budget 5 the best we can do is
        // "日本" (4) + "…" (1) = 5.
        assert_eq!(truncate_end("日本語", 5), "日本\u{2026}");
    }

    #[test]
    fn truncate_end_max_cols_zero_returns_empty() {
        assert_eq!(truncate_end("hello", 0), "");
    }

    #[test]
    fn truncate_end_max_cols_one_returns_ellipsis_if_input_wider() {
        assert_eq!(truncate_end("hello", 1), "\u{2026}");
    }
}
