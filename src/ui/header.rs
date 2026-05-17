//! Main pane title header — the compact `repo/branch ● N files ● -del/+ins ●
//! ↑↓ ● stash ● <mode>` line that lives in the rounded top border via
//! `Block::title`.
//!
//! When the pane is too narrow for the full title, long branch and repo
//! names are mid-ellipsized first (down to a per-part floor), then
//! lower-priority segments are dropped along with their preceding
//! separator.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::state::{AppState, ViewMode};
use crate::theme::Theme;
use crate::ui::fit;

/// Minimum display width to keep after mid-ellipsizing a branch name.
const BRANCH_FLOOR: usize = 12;
/// Minimum display width for the `repo/` prefix (includes the trailing `/`).
const REPO_PREFIX_FLOOR: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentKind {
    /// Always kept. Never dropped.
    Fixed,
    /// Droppable; lower priority number = dropped first.
    Droppable(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentRole {
    /// A separator that belongs before a content segment.
    Separator,
    /// The `<repo>/` prefix of the repo/branch pair.
    RepoPrefix,
    /// The branch name.
    Branch,
    /// Any other content segment.
    Content,
}

#[derive(Debug, Clone)]
struct Segment {
    kind: SegmentKind,
    role: SegmentRole,
    spans: Vec<Span<'static>>,
    text_width: usize,
}

impl Segment {
    fn new(kind: SegmentKind, role: SegmentRole, spans: Vec<Span<'static>>) -> Self {
        let text_width: usize = spans
            .iter()
            .map(|s| fit::display_width(s.content.as_ref()))
            .sum();
        Self {
            kind,
            role,
            spans,
            text_width,
        }
    }
}

/// Back-compat entry point for callers that don't know the available width.
pub fn build_header_title(state: &AppState, theme: &Theme) -> Line<'static> {
    build_header_title_with_width(state, theme, u16::MAX as usize)
}

/// Width-aware builder. `max_width` is the inner budget available for
/// the title content (caller should pass
/// `main_area.width.saturating_sub(2)` to leave room for rounded-border
/// decorations).
pub fn build_header_title_with_width(
    state: &AppState,
    theme: &Theme,
    max_width: usize,
) -> Line<'static> {
    if let Some(msg) = state.flash_message() {
        let inner_budget = max_width.saturating_sub(2);
        let clipped = fit::truncate_end(msg, inner_budget);
        return Line::from(vec![
            Span::raw(" "),
            Span::styled(clipped.into_owned(), Style::default().fg(theme.header_text)),
            Span::raw(" "),
        ]);
    }

    let repo = state.repo_name().to_string();
    let branch = state.branch().to_string();
    let segments = build_segments(state, &repo, &branch, theme);
    let fitted = fit_segments(segments, max_width, theme);

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    for seg in fitted {
        spans.extend(seg.spans);
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn build_segments(state: &AppState, repo: &str, branch: &str, theme: &Theme) -> Vec<Segment> {
    let text_style = Style::default().fg(theme.header_text);
    let sep_style = Style::default().fg(theme.header_separator);
    let sep_span = || Span::styled(" ● ", sep_style);

    let files = state.files();
    let file_count = files.len();
    let total_ins: usize = files.iter().map(|f| f.insertions).sum();
    let total_del: usize = files.iter().map(|f| f.deletions).sum();

    let mut out: Vec<Segment> = Vec::new();

    // Leading "repo/branch" segment.
    match (repo.is_empty(), branch.is_empty()) {
        (false, false) => {
            // "repo/" is droppable; the branch name itself is Fixed.
            let repo_prefix = format!("{repo}/");
            out.push(Segment::new(
                SegmentKind::Droppable(4),
                SegmentRole::RepoPrefix,
                vec![Span::styled(repo_prefix, text_style)],
            ));
            out.push(Segment::new(
                SegmentKind::Fixed,
                SegmentRole::Branch,
                vec![Span::styled(branch.to_string(), text_style)],
            ));
        }
        (true, false) => {
            out.push(Segment::new(
                SegmentKind::Fixed,
                SegmentRole::Branch,
                vec![Span::styled(branch.to_string(), text_style)],
            ));
        }
        (false, true) => {
            out.push(Segment::new(
                SegmentKind::Fixed,
                SegmentRole::Content,
                vec![Span::styled(repo.to_string(), text_style)],
            ));
        }
        (true, true) => {}
    }

    // Resolved diff base — the branch this worktree's changes are measured
    // against.
    // Droppable(0) — the lowest priority number, so it is dropped first.
    let base = state.base_branch();
    if !base.is_empty() {
        push_sep(&mut out, sep_span());
        out.push(Segment::new(
            SegmentKind::Droppable(0),
            SegmentRole::Content,
            vec![Span::styled(format!("base {base}"), text_style)],
        ));
    }

    push_sep(&mut out, sep_span());
    out.push(Segment::new(
        SegmentKind::Droppable(5),
        SegmentRole::Content,
        vec![Span::styled(format!("{file_count} files"), text_style)],
    ));

    if total_ins > 0 || total_del > 0 {
        push_sep(&mut out, sep_span());
        out.push(Segment::new(
            SegmentKind::Droppable(3),
            SegmentRole::Content,
            vec![
                Span::styled(
                    format!("-{total_del}"),
                    Style::default().fg(theme.file_deletions),
                ),
                Span::styled("/", text_style),
                Span::styled(
                    format!("+{total_ins}"),
                    Style::default().fg(theme.file_insertions),
                ),
            ],
        ));
    }

    if let Some((ahead, behind)) = state.ahead_behind() {
        if ahead > 0 || behind > 0 {
            push_sep(&mut out, sep_span());
            out.push(Segment::new(
                SegmentKind::Droppable(2),
                SegmentRole::Content,
                vec![Span::styled(format!("↑{ahead} ↓{behind}"), text_style)],
            ));
        }
    }

    let stash = state.stash_count();
    if stash > 0 {
        push_sep(&mut out, sep_span());
        out.push(Segment::new(
            SegmentKind::Droppable(1),
            SegmentRole::Content,
            vec![Span::styled(format!("{stash} stash"), text_style)],
        ));
    }

    push_sep(&mut out, sep_span());
    out.push(Segment::new(
        SegmentKind::Fixed,
        SegmentRole::Content,
        vec![Span::styled(
            match state.view_mode() {
                ViewMode::Flat => "flat",
                ViewMode::Tree => "tree",
            },
            text_style,
        )],
    ));

    out
}

fn push_sep(segs: &mut Vec<Segment>, sep: Span<'static>) {
    // Only emit a separator when there's already content to separate
    // from, and the previous segment isn't itself a separator.
    match segs.last() {
        None => return,
        Some(last) if last.role == SegmentRole::Separator => return,
        _ => {}
    }
    segs.push(Segment::new(
        SegmentKind::Fixed,
        SegmentRole::Separator,
        vec![sep],
    ));
}

fn total_width(segs: &[Segment]) -> usize {
    segs.iter().map(|s| s.text_width).sum::<usize>() + 2 /* leading + trailing " " */
}

fn fit_segments(mut segs: Vec<Segment>, max_width: usize, theme: &Theme) -> Vec<Segment> {
    if total_width(&segs) <= max_width {
        return segs;
    }

    // Step 1: ellipsize branch down to BRANCH_FLOOR.
    shrink_segment(
        &mut segs,
        SegmentRole::Branch,
        max_width,
        BRANCH_FLOOR,
        theme,
    );
    if total_width(&segs) <= max_width {
        return segs;
    }

    // Step 2: ellipsize "repo/" prefix down to REPO_PREFIX_FLOOR.
    shrink_segment(
        &mut segs,
        SegmentRole::RepoPrefix,
        max_width,
        REPO_PREFIX_FLOOR,
        theme,
    );
    if total_width(&segs) <= max_width {
        return segs;
    }

    // Step 3: drop droppable segments in ascending priority order, along
    // with their preceding separator.
    for priority in 0..=5 {
        drop_segment_with_priority(&mut segs, priority);
        if total_width(&segs) <= max_width {
            return segs;
        }
    }
    segs
}

fn shrink_segment(
    segs: &mut [Segment],
    role: SegmentRole,
    max_width: usize,
    floor: usize,
    theme: &Theme,
) {
    let current_total = total_width(segs);
    if current_total <= max_width {
        return;
    }
    let Some(idx) = segs.iter().position(|s| s.role == role) else {
        return;
    };
    let seg_width = segs[idx].text_width;
    if seg_width <= floor {
        return;
    }

    let over = current_total - max_width;
    let desired = seg_width.saturating_sub(over).max(floor);

    // Segment spans contain plain text — join and re-ellipsize.
    let original: String = segs[idx]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    let ellipsized = fit::middle_ellipsize(&original, desired);
    if ellipsized.as_ref() == original {
        return;
    }
    let new_text = ellipsized.into_owned();
    let new_width = fit::display_width(&new_text);
    let style = Style::default().fg(theme.header_text);
    segs[idx].spans = vec![Span::styled(new_text, style)];
    segs[idx].text_width = new_width;
}

fn drop_segment_with_priority(segs: &mut Vec<Segment>, priority: u8) {
    let Some(idx) = segs
        .iter()
        .position(|s| s.kind == SegmentKind::Droppable(priority))
    else {
        return;
    };
    // Drop the segment itself.
    segs.remove(idx);
    // Drop the preceding separator if present.
    if idx > 0 && segs[idx - 1].role == SegmentRole::Separator {
        segs.remove(idx - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fit::display_width;
    use std::time::Duration;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    fn test_theme() -> Theme {
        crate::theme::load_theme(crate::theme::DEFAULT_THEME_NAME, None)
    }

    fn fresh_state() -> AppState {
        let mut s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        s.set_repo_name("perch".to_string());
        s.set_worktree_name("perch".to_string());
        s
    }

    #[test]
    fn test_header_title_basic() {
        let s = fresh_state();
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " perch/main ● 0 files ● flat ");
    }

    #[test]
    fn test_header_title_with_file_stats() {
        use crate::git::{FileEntry, FileStatus};
        let mut s = fresh_state();
        s.update_files(vec![
            FileEntry {
                path: "a.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 12,
                deletions: 3,
            },
            FileEntry {
                path: "b.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 4,
                deletions: 0,
            },
        ]);
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " perch/main ● 2 files ● -3/+16 ● flat ");
    }

    #[test]
    fn test_header_title_with_ab_and_stash() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((2, 1)));
        s.set_stash_count(3);
        let line = build_header_title(&s, &test_theme());
        assert_eq!(
            line_text(&line),
            " perch/main ● 0 files ● ↑2 ↓1 ● 3 stash ● flat "
        );
    }

    #[test]
    fn test_header_title_hides_zero_ab() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((0, 0)));
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " perch/main ● 0 files ● flat ");
    }

    #[test]
    fn test_header_title_hides_zero_stash() {
        let mut s = fresh_state();
        s.set_stash_count(0);
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " perch/main ● 0 files ● flat ");
    }

    #[test]
    fn test_header_title_flash_message_replaces_content() {
        let mut s = fresh_state();
        s.set_flash_message("Switched to worktree: foo".to_string());
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " Switched to worktree: foo ");
    }

    #[test]
    fn test_header_title_empty_repo_name_shows_branch_only() {
        let s = AppState::new(vec![], Duration::from_millis(600), "main".to_string());
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " main ● 0 files ● flat ");
    }

    #[test]
    fn test_header_title_detached_head_shows_repo_only() {
        let mut s = AppState::new(vec![], Duration::from_millis(600), String::new());
        s.set_repo_name("perch".to_string());
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " perch ● 0 files ● flat ");
    }

    #[test]
    fn test_header_title_no_repo_no_branch_omits_segment() {
        let s = AppState::new(vec![], Duration::from_millis(600), String::new());
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " 0 files ● flat ");
    }

    #[test]
    fn test_header_title_includes_view_mode_label() {
        let mut s = fresh_state();
        s.cycle_view_mode();
        let line = build_header_title(&s, &test_theme());
        assert!(line_text(&line).contains("tree"));
    }

    #[test]
    fn test_header_drops_stash_first() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((2, 1)));
        s.set_stash_count(3);
        let full = line_text(&build_header_title(&s, &test_theme()));
        let full_w = display_width(&full);
        let line = build_header_title_with_width(&s, &test_theme(), full_w - 1);
        let text = line_text(&line);
        assert!(
            !text.contains("stash"),
            "stash should drop first, got: {text}"
        );
        assert!(
            text.contains("↑2 ↓1"),
            "ahead/behind should remain, got: {text}"
        );
    }

    #[test]
    fn test_header_drops_ahead_behind_second() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((2, 1)));
        s.set_stash_count(3);
        let line = build_header_title_with_width(&s, &test_theme(), 30);
        let text = line_text(&line);
        assert!(!text.contains("stash"), "got: {text}");
        assert!(!text.contains('↑'), "got: {text}");
    }

    #[test]
    fn test_header_keeps_view_mode_label_even_when_tight() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((2, 1)));
        s.set_stash_count(3);
        let line = build_header_title_with_width(&s, &test_theme(), 20);
        let text = line_text(&line);
        assert!(
            text.contains("flat"),
            "view-mode label must remain, got: {text}"
        );
    }

    #[test]
    fn test_header_mid_ellipsizes_long_branch_at_narrow_width() {
        let mut s = fresh_state();
        s.set_branch("feat/very-long-branch-name-with-lots-of-words".to_string());
        let line = build_header_title_with_width(&s, &test_theme(), 40);
        let text = line_text(&line);
        assert!(text.contains('\u{2026}'), "expected ellipsis, got: {text}");
        assert!(text.contains("perch"), "got: {text}");
        assert!(
            display_width(&text) <= 40,
            "got width {}: {text}",
            display_width(&text)
        );
    }

    #[test]
    fn test_header_shows_base_segment_when_set() {
        let mut s = fresh_state();
        s.set_branch("agent-work".to_string());
        s.set_merge_base(None, "main".to_string());
        let line = build_header_title_with_width(&s, &test_theme(), 200);
        assert!(
            line_text(&line).contains("base main"),
            "expected 'base main' in header, got: {:?}",
            line_text(&line)
        );
    }

    #[test]
    fn test_header_omits_base_segment_when_unset() {
        let mut s = fresh_state();
        s.set_branch("agent-work".to_string());
        // base_branch defaults to "" — no segment.
        let line = build_header_title_with_width(&s, &test_theme(), 200);
        assert!(
            !line_text(&line).contains("base "),
            "expected no base segment, got: {:?}",
            line_text(&line)
        );
    }

    #[test]
    fn test_header_drops_base_segment_first_when_narrow() {
        let mut s = fresh_state();
        s.set_branch("agent-work".to_string());
        s.set_merge_base(None, "main".to_string());
        let full = build_header_title_with_width(&s, &test_theme(), 200);
        let full_w = display_width(&line_text(&full));
        let narrow = build_header_title_with_width(&s, &test_theme(), full_w - 1);
        assert!(
            !line_text(&narrow).contains("base main"),
            "base segment should drop first, got: {:?}",
            line_text(&narrow)
        );
        assert!(
            line_text(&narrow).contains("agent-work"),
            "branch should still be visible, got: {:?}",
            line_text(&narrow)
        );
    }
}
