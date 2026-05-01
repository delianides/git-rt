//! Small wrapper around `nucleo-matcher`.
//!
//! Scoped to the matching primitive only — no UI. Intended to be reused by
//! future fuzzy use cases (file-list filter, branch picker) without
//! committing to a shared modal-picker widget yet.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Utf32Str};

/// Reusable fuzzy matcher. Reuse a single instance across keystrokes — the
/// inner matcher allocates internal scratch buffers that we want to keep.
pub struct Matcher {
    inner: nucleo_matcher::Matcher,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Matcher {
    pub fn new() -> Self {
        Self {
            inner: nucleo_matcher::Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    /// Rank `items` against `query`, returning indices into `items` sorted by
    /// descending match score. An empty query returns `0..items.len()` in
    /// original order. Items that do not match are omitted.
    ///
    /// `label` extracts the searchable string from each item.
    ///
    /// Note: a fresh `Pattern` is parsed on every call. The inner matcher's
    /// scratch buffers are reused across calls (the whole reason `Matcher`
    /// is held in state), but the per-keystroke `Pattern::parse` allocation
    /// is unavoidable with the current API. If profiling ever shows this is
    /// hot, cache the `(query, Pattern)` pair inside `Matcher`.
    pub fn rank<T>(&mut self, query: &str, items: &[T], label: impl Fn(&T) -> &str) -> Vec<usize> {
        if query.is_empty() {
            return (0..items.len()).collect();
        }

        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);

        let mut scratch = Vec::new();
        let mut scored: Vec<(usize, u32)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let l = label(item);
                let needle = Utf32Str::new(l, &mut scratch);
                pattern.score(needle, &mut self.inner).map(|s| (i, s))
            })
            .collect();

        // Highest score first. Ties keep input order via stable sort.
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(i, _)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_all_in_order() {
        let mut m = Matcher::new();
        let items = ["alpha", "beta", "gamma"];
        let ranked = m.rank("", &items, |s: &&str| *s);
        assert_eq!(ranked, vec![0, 1, 2]);
    }

    #[test]
    fn no_matches_returns_empty() {
        let mut m = Matcher::new();
        let items = ["alpha", "beta"];
        let ranked = m.rank("zzz", &items, |s: &&str| *s);
        assert!(ranked.is_empty());
    }

    #[test]
    fn ranks_clearer_match_higher() {
        let mut m = Matcher::new();
        let items = ["abracadabra", "abc"];
        // "abc" is a tighter match for the query "abc".
        let ranked = m.rank("abc", &items, |s: &&str| *s);
        assert_eq!(ranked.first().copied(), Some(1));
    }

    #[test]
    fn case_insensitive_query() {
        // Smart-case: lowercase query matches case-insensitively.
        let mut m = Matcher::new();
        let items = ["Alpha", "BETA"];
        let ranked = m.rank("beta", &items, |s: &&str| *s);
        assert_eq!(ranked.first().copied(), Some(1));
    }

    #[test]
    fn fuzzy_subsequence_matches() {
        let mut m = Matcher::new();
        let items = ["fix/diff-overlay", "feat/tree-modes", "main"];
        // "fdov" should subsequence-match the first item.
        let ranked = m.rank("fdov", &items, |s: &&str| *s);
        assert_eq!(ranked.first().copied(), Some(0));
    }
}
