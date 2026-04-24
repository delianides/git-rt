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

/// Mid-ellipsis shrinking. Returns `s` unchanged when it fits.
/// Otherwise produces `head…tail` with total display width ≤ `max_cols`.
///
/// This char-level version biases toward preserving the tail (often
/// the most-identifying substring — e.g. a filename). Task 4 layers
/// path-segment awareness on top of this for inputs containing `/`.
pub fn middle_ellipsize(s: &str, max_cols: usize) -> Cow<'_, str> {
    if max_cols == 0 {
        return Cow::Borrowed("");
    }
    if display_width(s) <= max_cols {
        return Cow::Borrowed(s);
    }
    if max_cols <= ELLIPSIS_WIDTH {
        return Cow::Owned(ELLIPSIS.to_string());
    }

    if s.contains('/') {
        if let Some(path_fit) = ellipsize_path(s, max_cols) {
            return Cow::Owned(path_fit);
        }
    }

    Cow::Owned(char_level_middle(s, max_cols))
}

fn char_level_middle(s: &str, max_cols: usize) -> String {
    // Reserve one column for the ellipsis; split remaining budget
    // between head and tail with a tail bias (favour filename).
    let available = max_cols - ELLIPSIS_WIDTH;
    let tail_budget = available.div_ceil(2);
    let head_budget = available - tail_budget;

    let head = take_prefix_cols(s, head_budget);
    let tail = take_suffix_cols(s, tail_budget);

    let mut out = String::with_capacity(head.len() + ELLIPSIS.len() + tail.len());
    out.push_str(head);
    out.push_str(ELLIPSIS);
    out.push_str(tail);
    out
}

/// Try to fit a path by dropping interior directory segments first.
/// Returns `None` when the head+tail+separators can't fit — caller
/// should fall back to char-level.
fn ellipsize_path(s: &str, max_cols: usize) -> Option<String> {
    let segments: Vec<&str> = s.split('/').collect();
    if segments.len() < 3 {
        return None;
    }
    let first = segments[0];
    let last = *segments.last().unwrap();

    // Minimum viable: "<first>/…/<last>"
    let minimal = build_path_with_keep(&segments, 1, 1);
    if display_width(&minimal) > max_cols {
        return None;
    }

    // Greedily re-add interior segments from the outside inward.
    let mut left_keep = 1usize;
    let mut right_keep = 1usize;
    loop {
        let mut progress = false;

        let proposed_left = left_keep + 1;
        if proposed_left + right_keep < segments.len() {
            let candidate = build_path_with_keep(&segments, proposed_left, right_keep);
            if display_width(&candidate) <= max_cols {
                left_keep = proposed_left;
                progress = true;
            }
        }

        let proposed_right = right_keep + 1;
        if left_keep + proposed_right < segments.len() {
            let candidate = build_path_with_keep(&segments, left_keep, proposed_right);
            if display_width(&candidate) <= max_cols {
                right_keep = proposed_right;
                progress = true;
            }
        }

        if !progress {
            break;
        }
    }

    // Silence unused-var lints on `first`/`last` (they were informational).
    let _ = (first, last);
    Some(build_path_with_keep(&segments, left_keep, right_keep))
}

fn build_path_with_keep(segments: &[&str], left: usize, right: usize) -> String {
    let left_part = segments[..left].join("/");
    let right_part = segments[segments.len() - right..].join("/");
    format!("{left_part}/{ELLIPSIS}/{right_part}")
}

fn take_prefix_cols(s: &str, budget: usize) -> &str {
    if budget == 0 {
        return "";
    }
    let mut used = 0usize;
    let mut end = 0usize;
    for (idx, ch) in s.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        end = idx + ch.len_utf8();
    }
    &s[..end]
}

fn take_suffix_cols(s: &str, budget: usize) -> &str {
    if budget == 0 {
        return "";
    }
    let mut used = 0usize;
    let mut start = s.len();
    for (idx, ch) in s.char_indices().rev() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        start = idx;
    }
    &s[start..]
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

    #[test]
    fn middle_ellipsize_already_fits_returns_unchanged() {
        assert_eq!(middle_ellipsize("abc", 5), "abc");
    }

    #[test]
    fn middle_ellipsize_exact_fit_returns_unchanged() {
        assert_eq!(middle_ellipsize("abcdef", 6), "abcdef");
    }

    #[test]
    fn middle_ellipsize_char_level_balances_head_and_tail() {
        // "abcdefghij" (10 cols) to budget 7 -> 3 head + "…" + 3 tail = 7.
        assert_eq!(middle_ellipsize("abcdefghij", 7), "abc\u{2026}hij");
    }

    #[test]
    fn middle_ellipsize_char_level_odd_budget_biases_tail() {
        // Budget 6, input 10 cols. Available = 5 → tail gets 3, head gets 2.
        assert_eq!(middle_ellipsize("abcdefghij", 6), "ab\u{2026}hij");
    }

    #[test]
    fn middle_ellipsize_budget_zero_returns_empty() {
        assert_eq!(middle_ellipsize("abc", 0), "");
    }

    #[test]
    fn middle_ellipsize_budget_one_returns_ellipsis() {
        assert_eq!(middle_ellipsize("abcdef", 1), "\u{2026}");
    }

    #[test]
    fn middle_ellipsize_budget_two_returns_tail_char_plus_ellipsis() {
        // Budget 2: "…" + tail char (1 col) = 2 cols. Head gets 0.
        assert_eq!(middle_ellipsize("abcdef", 2), "\u{2026}f");
    }

    #[test]
    fn middle_ellipsize_path_preserves_filename_tail() {
        // "src/ui/deep/nested/file.rs" is 25 cols; budget 16 should
        // keep "file.rs" at the tail and "src" at the head.
        let out = middle_ellipsize("src/ui/deep/nested/file.rs", 16);
        assert!(out.ends_with("file.rs"), "got: {out}");
        assert!(out.starts_with("src"), "got: {out}");
        assert!(out.contains('\u{2026}'), "got: {out}");
        assert!(display_width(&out) <= 16, "got width: {}", display_width(&out));
    }

    #[test]
    fn middle_ellipsize_path_drops_interior_segments_from_middle() {
        let out = middle_ellipsize("src/ui/mod.rs", 12);
        assert!(out.ends_with("mod.rs"), "got: {out}");
        assert!(display_width(&out) <= 12, "got width: {}", display_width(&out));
    }

    #[test]
    fn middle_ellipsize_path_falls_back_to_char_level_when_no_interior_fits() {
        // "ab/c" (4 cols) at budget 3 — only 2 segments, path logic
        // returns None and char-level takes over.
        let out = middle_ellipsize("ab/c", 3);
        assert!(display_width(&out) <= 3, "got width: {}", display_width(&out));
    }

    #[test]
    fn middle_ellipsize_non_path_input_uses_char_level() {
        assert_eq!(middle_ellipsize("abcdefghij", 7), "abc\u{2026}hij");
    }
}
