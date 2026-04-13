//! Main pane title header — the compact `repo/branch · N files +ins -del ·
//! ↑↓ · stash` line that lives in the rounded top border via
//! `Block::title`.
//!
//! When a flash message is active (e.g., "Switched to worktree: X"),
//! the title is replaced with the flash text for the flash duration so
//! the message is visible against the pane border.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::state::AppState;
use crate::theme::Theme;

/// Build the main pane's title `Line`.
pub fn build_header_title(state: &AppState, theme: &Theme) -> Line<'static> {
    // Flash message takes over the title for its duration.
    if let Some(msg) = state.flash_message() {
        return Line::from(vec![
            Span::raw(" "),
            Span::styled(msg.to_string(), Style::default().fg(theme.header_text)),
            Span::raw(" "),
        ]);
    }

    let files = state.files();
    let file_count = files.len();
    let total_ins: usize = files.iter().map(|f| f.insertions).sum();
    let total_del: usize = files.iter().map(|f| f.deletions).sum();

    let repo = state.repo_name();
    let branch = state.branch();

    let text_style = Style::default().fg(theme.header_text);
    let sep_style = Style::default().fg(theme.header_separator);

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];

    // Leading `repo/branch` segment (with graceful fallbacks).
    let leading = match (repo.is_empty(), branch.is_empty()) {
        (false, false) => Some(format!("{repo}/{branch}")),
        (true, false) => Some(branch.to_string()),
        (false, true) => Some(repo.to_string()),
        (true, true) => None,
    };
    if let Some(text) = leading {
        spans.push(Span::styled(text, text_style));
        spans.push(Span::styled(" · ", sep_style));
    }

    // file count + diff stats (always shown)
    spans.push(Span::styled(format!("{file_count} files"), text_style));
    if total_ins > 0 || total_del > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("+{total_ins}"),
            Style::default().fg(theme.file_insertions),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{total_del}"),
            Style::default().fg(theme.file_deletions),
        ));
    }

    // ahead/behind (only when non-zero)
    if let Some((ahead, behind)) = state.ahead_behind() {
        if ahead > 0 || behind > 0 {
            spans.push(Span::styled(" · ", sep_style));
            spans.push(Span::styled(format!("↑{ahead} ↓{behind}"), text_style));
        }
    }

    // stash count (only when > 0)
    let stash = state.stash_count();
    if stash > 0 {
        spans.push(Span::styled(" · ", sep_style));
        spans.push(Span::styled(format!("{stash} stash"), text_style));
    }

    spans.push(Span::raw(" "));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        s.set_repo_name("git-rt".to_string());
        s.set_worktree_name("git-rt".to_string());
        s
    }

    #[test]
    fn test_header_title_basic() {
        let s = fresh_state();
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " git-rt/main · 0 files ");
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
        assert_eq!(line_text(&line), " git-rt/main · 2 files +16 -3 ");
    }

    #[test]
    fn test_header_title_with_ab_and_stash() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((2, 1)));
        s.set_stash_count(3);
        let line = build_header_title(&s, &test_theme());
        assert_eq!(
            line_text(&line),
            " git-rt/main · 0 files · ↑2 ↓1 · 3 stash "
        );
    }

    #[test]
    fn test_header_title_hides_zero_ab() {
        let mut s = fresh_state();
        s.set_ahead_behind(Some((0, 0)));
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " git-rt/main · 0 files ");
    }

    #[test]
    fn test_header_title_hides_zero_stash() {
        let mut s = fresh_state();
        s.set_stash_count(0);
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " git-rt/main · 0 files ");
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
        // repo_name left empty
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " main · 0 files ");
    }

    #[test]
    fn test_header_title_detached_head_shows_repo_only() {
        let mut s = AppState::new(vec![], Duration::from_millis(600), String::new());
        s.set_repo_name("git-rt".to_string());
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " git-rt · 0 files ");
    }

    #[test]
    fn test_header_title_no_repo_no_branch_omits_segment() {
        let s = AppState::new(vec![], Duration::from_millis(600), String::new());
        let line = build_header_title(&s, &test_theme());
        assert_eq!(line_text(&line), " 0 files ");
    }
}
