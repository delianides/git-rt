use std::collections::HashMap;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::config::ColorValue;
use crate::git::FileStatus;
use crate::state::AppState;

/// A parsed segment of a statusline format string
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusSegment {
    /// A data token like %b, %c, %+, etc.
    Token(char),
    /// Literal text
    Literal(String),
    /// Right-align marker (%=)
    RightAlign,
    /// Start of a color/modifier tag
    StyleStart(StyleTag),
    /// End of the innermost style tag ({/})
    StyleEnd,
}

/// A style tag from {color_name} or {bold}/{dim}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleTag {
    Color(String), // Named color or hex like "#FF8800"
    Bold,
    Dim,
}

/// Parse a statusline format string into segments.
/// Handles %tokens, {color}...{/} style tags, and %= right-align.
pub fn parse_status_format(fmt: &str) -> Vec<StatusSegment> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = fmt.chars().peekable();
    let mut seen_right_align = false;

    while let Some(ch) = chars.next() {
        match ch {
            '%' => {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    match next {
                        '%' => {
                            // Escaped percent — add to literal
                            literal.push('%');
                        }
                        '=' if !seen_right_align => {
                            if !literal.is_empty() {
                                segments.push(StatusSegment::Literal(literal.clone()));
                                literal.clear();
                            }
                            segments.push(StatusSegment::RightAlign);
                            seen_right_align = true;
                        }
                        '=' => {
                            literal.push('%');
                            literal.push('=');
                        }
                        other => {
                            if !literal.is_empty() {
                                segments.push(StatusSegment::Literal(literal.clone()));
                                literal.clear();
                            }
                            segments.push(StatusSegment::Token(other));
                        }
                    }
                } else {
                    // Trailing %, treat as literal
                    literal.push('%');
                }
            }
            '{' => {
                // Read until closing '}'
                let mut tag_content = String::new();
                let mut found_close = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        found_close = true;
                        break;
                    }
                    tag_content.push(c);
                }
                if found_close {
                    // Flush accumulated literal
                    if !literal.is_empty() {
                        segments.push(StatusSegment::Literal(literal.clone()));
                        literal.clear();
                    }
                    match tag_content.as_str() {
                        "/" => segments.push(StatusSegment::StyleEnd),
                        "bold" => segments.push(StatusSegment::StyleStart(StyleTag::Bold)),
                        "dim" => segments.push(StatusSegment::StyleStart(StyleTag::Dim)),
                        other => segments.push(StatusSegment::StyleStart(StyleTag::Color(
                            other.to_string(),
                        ))),
                    }
                } else {
                    // Unclosed brace, treat as literal
                    literal.push('{');
                    literal.push_str(&tag_content);
                }
            }
            other => {
                literal.push(other);
            }
        }
    }

    // Flush remaining literal
    if !literal.is_empty() {
        segments.push(StatusSegment::Literal(literal));
    }

    segments
}

/// Resolve a status format token to its string value.
/// Returns empty string for tokens that should render nothing when zero/empty.
pub fn resolve_status_token(token: char, state: &AppState) -> String {
    match token {
        'R' => state.repo_name().to_string(),
        'b' => state.branch().to_string(),
        'w' => {
            let name = state.worktree_name();
            if name.is_empty() {
                String::new()
            } else {
                name.to_string()
            }
        }
        'c' => state.files().len().to_string(),
        '+' => {
            let total: usize = state.files().iter().map(|f| f.insertions).sum();
            if total == 0 {
                String::new()
            } else {
                format!("+{total}")
            }
        }
        '-' => {
            let total: usize = state.files().iter().map(|f| f.deletions).sum();
            if total == 0 {
                String::new()
            } else {
                format!("-{total}")
            }
        }
        't' => {
            let total: usize = state
                .files()
                .iter()
                .map(|f| f.insertions + f.deletions)
                .sum();
            if total == 0 {
                String::new()
            } else {
                total.to_string()
            }
        }
        'u' => {
            let count = state
                .files()
                .iter()
                .filter(|f| f.status == FileStatus::Untracked)
                .count();
            if count == 0 {
                String::new()
            } else {
                count.to_string()
            }
        }
        's' => {
            let count = state
                .files()
                .iter()
                .filter(|f| f.status == FileStatus::Staged)
                .count();
            if count == 0 {
                String::new()
            } else {
                count.to_string()
            }
        }
        'm' => {
            let count = state
                .files()
                .iter()
                .filter(|f| f.status == FileStatus::Modified)
                .count();
            if count == 0 {
                String::new()
            } else {
                count.to_string()
            }
        }
        'S' => {
            let count = state.stash_count();
            if count == 0 {
                String::new()
            } else {
                count.to_string()
            }
        }
        'a' => match state.ahead_behind() {
            Some((0, 0)) | None => String::new(),
            Some((ahead, behind)) => format!("\u{2191}{ahead} \u{2193}{behind}"),
        },
        'g' => state.repo_state().unwrap_or("").to_string(),
        'H' => state.head_sha().to_string(),
        'M' => state.head_message().to_string(),
        'r' => {
            let count = state.refresh_count();
            if count == 0 {
                String::new()
            } else {
                format!("#{count}")
            }
        }
        'T' => {
            let secs = state.last_refresh_secs();
            if secs == 0 {
                String::new()
            } else {
                format!("{secs}s ago")
            }
        }
        'h' => "j/k:nav  enter:expand  q:quit".to_string(),
        _ => String::new(),
    }
}

/// The kind of an expanded segment (post-token-resolution)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpandedKind {
    Text,
    RightAlign,
    StyleStart(StyleTag),
    StyleEnd,
}

/// An expanded segment with resolved text
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedSegment {
    pub text: String,
    pub kind: ExpandedKind,
}

/// Expand tokens in a parsed status format, collapsing whitespace around empty tokens.
pub fn expand_status_line(segments: &[StatusSegment], state: &AppState) -> Vec<ExpandedSegment> {
    let mut raw: Vec<ExpandedSegment> = Vec::new();

    for segment in segments {
        match segment {
            StatusSegment::Token(ch) => {
                let value = resolve_status_token(*ch, state);
                raw.push(ExpandedSegment {
                    text: value,
                    kind: ExpandedKind::Text,
                });
            }
            StatusSegment::Literal(text) => {
                raw.push(ExpandedSegment {
                    text: text.clone(),
                    kind: ExpandedKind::Text,
                });
            }
            StatusSegment::RightAlign => {
                raw.push(ExpandedSegment {
                    text: String::new(),
                    kind: ExpandedKind::RightAlign,
                });
            }
            StatusSegment::StyleStart(tag) => {
                raw.push(ExpandedSegment {
                    text: String::new(),
                    kind: ExpandedKind::StyleStart(tag.clone()),
                });
            }
            StatusSegment::StyleEnd => {
                raw.push(ExpandedSegment {
                    text: String::new(),
                    kind: ExpandedKind::StyleEnd,
                });
            }
        }
    }

    collapse_whitespace(&mut raw);
    raw
}

/// Collapse whitespace around empty text segments.
/// When a text segment is empty, trim adjacent whitespace and collapse double spaces.
fn collapse_whitespace(segments: &mut Vec<ExpandedSegment>) {
    // Track which text segment indices were modified by empty-token trimming
    let mut modified_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Find indices of empty text segments
    let empty_indices: Vec<usize> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == ExpandedKind::Text && s.text.is_empty())
        .map(|(i, _)| i)
        .collect();

    // For each empty text segment, trim surrounding whitespace
    for &idx in &empty_indices {
        if let Some((pi, prev)) = segments[..idx]
            .iter_mut()
            .enumerate()
            .rev()
            .find(|(_, s)| s.kind == ExpandedKind::Text && !s.text.is_empty())
        {
            prev.text = prev.text.trim_end().to_string();
            modified_indices.insert(pi);
        }

        // Find the actual index in the full segments vec for the next non-empty text
        if let Some((offset, next)) = segments[idx + 1..]
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.kind == ExpandedKind::Text && !s.text.is_empty())
        {
            next.text = next.text.trim_start().to_string();
            modified_indices.insert(idx + 1 + offset);
        }
    }

    // Remove empty text segments
    segments.retain(|s| !(s.kind == ExpandedKind::Text && s.text.is_empty()));

    // After removing empty segments, check if adjacent non-empty text segments
    // need a space inserted between them (when both were trimmed and are now touching)
    let len = segments.len();
    let mut inserts: Vec<usize> = Vec::new();
    for i in 0..len.saturating_sub(1) {
        if segments[i].kind == ExpandedKind::Text
            && !segments[i].text.is_empty()
            && segments[i + 1].kind == ExpandedKind::Text
            && !segments[i + 1].text.is_empty()
        {
            let left_ends_space = segments[i].text.ends_with(' ');
            let right_starts_space = segments[i + 1].text.starts_with(' ');
            if !left_ends_space && !right_starts_space {
                // Check if there were empty tokens between these (i.e., both were trimmed)
                // We can detect this: if original had whitespace between them that got trimmed
                // A simpler heuristic: if either segment was modified, add a space
                // But we lost exact indices after retain. Instead, just check if neither has space.
                inserts.push(i + 1);
            }
        }
    }
    // Insert spaces (reverse order to preserve indices)
    for &pos in inserts.iter().rev() {
        segments.insert(
            pos,
            ExpandedSegment {
                text: " ".to_string(),
                kind: ExpandedKind::Text,
            },
        );
    }

    // Trim leading whitespace on first text segment
    if let Some(first) = segments
        .iter_mut()
        .find(|s| s.kind == ExpandedKind::Text && !s.text.is_empty())
    {
        first.text = first.text.trim_start().to_string();
    }
    // Trim trailing whitespace on last text segment
    if let Some(last) = segments
        .iter_mut()
        .rev()
        .find(|s| s.kind == ExpandedKind::Text && !s.text.is_empty())
    {
        last.text = last.text.trim_end().to_string();
    }

    // Final cleanup: remove any segments that became empty after trimming
    segments.retain(|s| !(s.kind == ExpandedKind::Text && s.text.is_empty()));
}

/// Resolve a style tag to a ratatui Style modification
fn resolve_style_tag(tag: &StyleTag, palette: &HashMap<String, ColorValue>) -> Style {
    match tag {
        StyleTag::Color(name) => {
            let color = if let Some(cv) = palette.get(&name.to_lowercase()) {
                cv.resolve()
            } else {
                ColorValue::new(name).resolve()
            };
            Style::default().fg(color)
        }
        StyleTag::Bold => Style::default().add_modifier(Modifier::BOLD),
        StyleTag::Dim => Style::default().add_modifier(Modifier::DIM),
    }
}

/// Render a status format line to a styled ratatui Line.
/// Handles color stack, right-alignment, and whitespace collapsing.
pub fn render_status_line<'a>(
    segments: &[StatusSegment],
    state: &AppState,
    available_width: u16,
    default_fg: Color,
    palette: &HashMap<String, ColorValue>,
) -> Line<'a> {
    let expanded = expand_status_line(segments, state);

    let right_align_pos = expanded
        .iter()
        .position(|s| s.kind == ExpandedKind::RightAlign);

    let (left_expanded, right_expanded) = match right_align_pos {
        Some(pos) => (&expanded[..pos], &expanded[pos + 1..]),
        None => (expanded.as_slice(), &[] as &[ExpandedSegment]),
    };

    let base_style = Style::default().fg(default_fg);

    let left_spans = build_styled_spans(left_expanded, base_style, palette);
    let right_spans = build_styled_spans(right_expanded, base_style, palette);

    if right_spans.is_empty() {
        return Line::from(left_spans);
    }

    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();
    let right_width: usize = right_spans.iter().map(|s| s.content.len()).sum();
    let spacer = (available_width as usize).saturating_sub(left_width + right_width);

    let mut spans = left_spans;
    spans.push(Span::raw(" ".repeat(spacer)));
    spans.extend(right_spans);

    Line::from(spans)
}

/// Build styled spans from expanded segments using a color/modifier stack
fn build_styled_spans<'a>(
    segments: &[ExpandedSegment],
    base_style: Style,
    palette: &HashMap<String, ColorValue>,
) -> Vec<Span<'a>> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();

    for segment in segments {
        match &segment.kind {
            ExpandedKind::Text => {
                if !segment.text.is_empty() {
                    let current_style = style_stack.last().copied().unwrap_or(base_style);
                    spans.push(Span::styled(segment.text.clone(), current_style));
                }
            }
            ExpandedKind::StyleStart(tag) => {
                let parent = style_stack.last().copied().unwrap_or(base_style);
                let tag_style = resolve_style_tag(tag, palette);
                let merged = merge_styles(parent, tag_style);
                style_stack.push(merged);
            }
            ExpandedKind::StyleEnd => {
                style_stack.pop();
            }
            ExpandedKind::RightAlign => {}
        }
    }

    spans
}

/// Merge two styles: child overrides parent for fg, and adds modifiers
fn merge_styles(parent: Style, child: Style) -> Style {
    let mut merged = parent;
    if let Some(fg) = child.fg {
        merged.fg = Some(fg);
    }
    if let Some(bg) = child.bg {
        merged.bg = Some(bg);
    }
    merged.add_modifier = merged.add_modifier.union(child.add_modifier);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_parse_simple_tokens() {
        let segments = parse_status_format("%b %c files");
        assert_eq!(
            segments,
            vec![
                StatusSegment::Token('b'),
                StatusSegment::Literal(" ".to_string()),
                StatusSegment::Token('c'),
                StatusSegment::Literal(" files".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_escaped_percent() {
        let segments = parse_status_format("100%%");
        assert_eq!(segments, vec![StatusSegment::Literal("100%".to_string())]);
    }

    #[test]
    fn test_parse_right_align() {
        let segments = parse_status_format("%b %=%R");
        assert_eq!(
            segments,
            vec![
                StatusSegment::Token('b'),
                StatusSegment::Literal(" ".to_string()),
                StatusSegment::RightAlign,
                StatusSegment::Token('R'),
            ]
        );
    }

    #[test]
    fn test_parse_color_tags() {
        let segments = parse_status_format("{red}%-{/}");
        assert_eq!(
            segments,
            vec![
                StatusSegment::StyleStart(StyleTag::Color("red".to_string())),
                StatusSegment::Token('-'),
                StatusSegment::StyleEnd,
            ]
        );
    }

    #[test]
    fn test_parse_hex_color() {
        let segments = parse_status_format("{#FF8800}text{/}");
        assert_eq!(
            segments,
            vec![
                StatusSegment::StyleStart(StyleTag::Color("#FF8800".to_string())),
                StatusSegment::Literal("text".to_string()),
                StatusSegment::StyleEnd,
            ]
        );
    }

    #[test]
    fn test_parse_bold_modifier() {
        let segments = parse_status_format("{bold}%b{/}");
        assert_eq!(
            segments,
            vec![
                StatusSegment::StyleStart(StyleTag::Bold),
                StatusSegment::Token('b'),
                StatusSegment::StyleEnd,
            ]
        );
    }

    #[test]
    fn test_parse_dim_modifier() {
        let segments = parse_status_format("{dim}%h{/}");
        assert_eq!(
            segments,
            vec![
                StatusSegment::StyleStart(StyleTag::Dim),
                StatusSegment::Token('h'),
                StatusSegment::StyleEnd,
            ]
        );
    }

    #[test]
    fn test_parse_nested_styles() {
        let segments = parse_status_format("{bold}{red}%-{/}{/}");
        assert_eq!(
            segments,
            vec![
                StatusSegment::StyleStart(StyleTag::Bold),
                StatusSegment::StyleStart(StyleTag::Color("red".to_string())),
                StatusSegment::Token('-'),
                StatusSegment::StyleEnd,
                StatusSegment::StyleEnd,
            ]
        );
    }

    #[test]
    fn test_parse_empty_string() {
        let segments = parse_status_format("");
        assert!(segments.is_empty());
    }

    #[test]
    fn test_parse_default_status_line() {
        let segments = parse_status_format("%b  %c files  {red}%-{/} {green}%+{/}  %=%R");
        assert_eq!(
            segments,
            vec![
                StatusSegment::Token('b'),
                StatusSegment::Literal("  ".to_string()),
                StatusSegment::Token('c'),
                StatusSegment::Literal(" files  ".to_string()),
                StatusSegment::StyleStart(StyleTag::Color("red".to_string())),
                StatusSegment::Token('-'),
                StatusSegment::StyleEnd,
                StatusSegment::Literal(" ".to_string()),
                StatusSegment::StyleStart(StyleTag::Color("green".to_string())),
                StatusSegment::Token('+'),
                StatusSegment::StyleEnd,
                StatusSegment::Literal("  ".to_string()),
                StatusSegment::RightAlign,
                StatusSegment::Token('R'),
            ]
        );
    }

    // -- Helper for token resolution and rendering tests --

    fn test_state() -> AppState {
        use crate::git::{FileEntry, FileStatus};
        let files = vec![
            FileEntry {
                path: "a.rs".to_string(),
                status: FileStatus::Modified,
                insertions: 10,
                deletions: 3,
            },
            FileEntry {
                path: "b.rs".to_string(),
                status: FileStatus::Untracked,
                insertions: 5,
                deletions: 0,
            },
            FileEntry {
                path: "c.rs".to_string(),
                status: FileStatus::Staged,
                insertions: 0,
                deletions: 2,
            },
        ];
        let mut state = AppState::new(
            files,
            std::time::Duration::from_millis(600),
            "main".to_string(),
        );
        state.set_repo_name("git-rt".to_string());
        state.set_worktree_name("my-worktree".to_string());
        state.set_head_info("abc1234".to_string(), "fix: a bug".to_string());
        state.set_stash_count(2);
        state.set_ahead_behind(Some((3, 1)));
        state.set_repo_state(Some("REBASING".to_string()));
        state
    }

    // -- Task 6: Token resolution tests --

    #[test]
    fn test_resolve_repo_name() {
        let state = test_state();
        assert_eq!(resolve_status_token('R', &state), "git-rt");
    }

    #[test]
    fn test_resolve_branch() {
        let state = test_state();
        assert_eq!(resolve_status_token('b', &state), "main");
    }

    #[test]
    fn test_resolve_worktree() {
        let state = test_state();
        assert_eq!(resolve_status_token('w', &state), "my-worktree");
    }

    #[test]
    fn test_resolve_file_count() {
        let state = test_state();
        assert_eq!(resolve_status_token('c', &state), "3");
    }

    #[test]
    fn test_resolve_total_insertions() {
        let state = test_state();
        assert_eq!(resolve_status_token('+', &state), "+15");
    }

    #[test]
    fn test_resolve_total_deletions() {
        let state = test_state();
        assert_eq!(resolve_status_token('-', &state), "-5");
    }

    #[test]
    fn test_resolve_total_changes() {
        let state = test_state();
        assert_eq!(resolve_status_token('t', &state), "20");
    }

    #[test]
    fn test_resolve_untracked_count() {
        let state = test_state();
        assert_eq!(resolve_status_token('u', &state), "1");
    }

    #[test]
    fn test_resolve_staged_count() {
        let state = test_state();
        assert_eq!(resolve_status_token('s', &state), "1");
    }

    #[test]
    fn test_resolve_modified_count() {
        let state = test_state();
        assert_eq!(resolve_status_token('m', &state), "1");
    }

    #[test]
    fn test_resolve_stash_count() {
        let state = test_state();
        assert_eq!(resolve_status_token('S', &state), "2");
    }

    #[test]
    fn test_resolve_ahead_behind() {
        let state = test_state();
        assert_eq!(resolve_status_token('a', &state), "\u{2191}3 \u{2193}1");
    }

    #[test]
    fn test_resolve_ahead_behind_synced_is_empty() {
        let mut state = test_state();
        state.set_ahead_behind(Some((0, 0)));
        assert_eq!(resolve_status_token('a', &state), "");
    }

    #[test]
    fn test_resolve_git_state() {
        let state = test_state();
        assert_eq!(resolve_status_token('g', &state), "REBASING");
    }

    #[test]
    fn test_resolve_head_sha() {
        let state = test_state();
        assert_eq!(resolve_status_token('H', &state), "abc1234");
    }

    #[test]
    fn test_resolve_head_message() {
        let state = test_state();
        assert_eq!(resolve_status_token('M', &state), "fix: a bug");
    }

    #[test]
    fn test_resolve_help() {
        let state = test_state();
        assert_eq!(
            resolve_status_token('h', &state),
            "j/k:nav  enter:expand  q:quit"
        );
    }

    #[test]
    fn test_resolve_empty_when_zero() {
        let state = AppState::new(
            vec![],
            std::time::Duration::from_millis(600),
            "main".to_string(),
        );
        assert_eq!(resolve_status_token('+', &state), "");
        assert_eq!(resolve_status_token('-', &state), "");
        assert_eq!(resolve_status_token('t', &state), "");
        assert_eq!(resolve_status_token('u', &state), "");
        assert_eq!(resolve_status_token('s', &state), "");
        assert_eq!(resolve_status_token('m', &state), "");
        assert_eq!(resolve_status_token('S', &state), "");
        assert_eq!(resolve_status_token('a', &state), "");
        assert_eq!(resolve_status_token('g', &state), "");
        assert_eq!(resolve_status_token('c', &state), "0");
    }

    // -- Task 7: Expansion tests --

    #[test]
    fn test_expand_simple() {
        let state = test_state();
        let segments = parse_status_format("%b  %c files");
        let expanded = expand_status_line(&segments, &state);
        let text: String = expanded.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "main  3 files");
    }

    #[test]
    fn test_expand_empty_token_collapses_whitespace() {
        let mut state = test_state();
        state.set_stash_count(0);
        let segments = parse_status_format("%b  %S  %c files");
        let expanded = expand_status_line(&segments, &state);
        let text: String = expanded.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "main 3 files");
    }

    #[test]
    fn test_expand_multiple_empty_tokens_collapse() {
        let mut state = test_state();
        state.set_stash_count(0);
        state.set_ahead_behind(None);
        let segments = parse_status_format("%b  %S  %a  %c");
        let expanded = expand_status_line(&segments, &state);
        let text: String = expanded.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "main 3");
    }

    #[test]
    fn test_expand_trims_leading_trailing() {
        let mut state = test_state();
        state.set_stash_count(0);
        let segments = parse_status_format("%S  %b");
        let expanded = expand_status_line(&segments, &state);
        let text: String = expanded.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "main");
    }

    #[test]
    fn test_expand_preserves_style_tags() {
        let state = test_state();
        let segments = parse_status_format("{red}%-{/}");
        let expanded = expand_status_line(&segments, &state);
        assert!(expanded
            .iter()
            .any(|s| matches!(s.kind, ExpandedKind::StyleStart(_))));
        assert!(expanded
            .iter()
            .any(|s| matches!(s.kind, ExpandedKind::StyleEnd)));
    }

    // -- Task 8: Rendering tests --

    #[test]
    fn test_render_plain_text() {
        let state = test_state();
        let segments = parse_status_format("%b  %c files");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("main"));
        assert!(text.contains("3 files"));
    }

    #[test]
    fn test_render_with_color() {
        let state = test_state();
        let segments = parse_status_format("{red}%-{/}");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let red_span = line.spans.iter().find(|s| s.content.contains("-5"));
        assert!(red_span.is_some());
        assert_eq!(red_span.unwrap().style.fg, Some(Color::Red));
    }

    #[test]
    fn test_render_with_hex_color() {
        let state = test_state();
        let segments = parse_status_format("{#FF0000}%-{/}");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let colored_span = line.spans.iter().find(|s| s.content.contains("-5"));
        assert!(colored_span.is_some());
        assert_eq!(colored_span.unwrap().style.fg, Some(Color::Rgb(255, 0, 0)));
    }

    #[test]
    fn test_render_with_bold() {
        let state = test_state();
        let segments = parse_status_format("{bold}%b{/}");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let bold_span = line.spans.iter().find(|s| s.content.contains("main"));
        assert!(bold_span.is_some());
        assert!(bold_span
            .unwrap()
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_render_default_fg_for_unstyled() {
        let state = test_state();
        let segments = parse_status_format("%b");
        let default_fg = Color::Cyan;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let span = line.spans.iter().find(|s| s.content.contains("main"));
        assert!(span.is_some());
        assert_eq!(span.unwrap().style.fg, Some(Color::Cyan));
    }

    #[test]
    fn test_render_right_align() {
        let state = test_state();
        let segments = parse_status_format("%b %=%R");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 40, default_fg, &HashMap::new());
        let total_width: usize = line.spans.iter().map(|s| s.content.len()).sum();
        assert_eq!(total_width, 40);
    }

    #[test]
    fn test_resolve_refresh_counter() {
        let mut state = test_state();
        // After update_files, refresh_count becomes 1
        state.update_files(state.files().to_vec());
        let result = resolve_status_token('r', &state);
        assert!(result.starts_with('#'));
    }

    #[test]
    fn test_resolve_refresh_counter_empty_when_zero() {
        let state = AppState::new(
            vec![],
            std::time::Duration::from_millis(600),
            "main".to_string(),
        );
        assert_eq!(resolve_status_token('r', &state), "");
    }

    #[test]
    fn test_resolve_empty_repo_name() {
        let state = AppState::new(
            vec![],
            std::time::Duration::from_millis(600),
            "main".to_string(),
        );
        // Default repo_name is empty
        assert_eq!(resolve_status_token('R', &state), "");
        assert_eq!(resolve_status_token('H', &state), "");
        assert_eq!(resolve_status_token('M', &state), "");
    }

    #[test]
    fn test_render_with_dim() {
        let state = test_state();
        let segments = parse_status_format("{dim}%h{/}");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let dim_span = line.spans.iter().find(|s| s.content.contains("j/k:nav"));
        assert!(dim_span.is_some());
        assert!(dim_span.unwrap().style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn test_render_nested_styles() {
        let state = test_state();
        let segments = parse_status_format("{bold}{red}%-{/}{/}");
        let default_fg = Color::White;
        let line = render_status_line(&segments, &state, 80, default_fg, &HashMap::new());
        let styled_span = line.spans.iter().find(|s| s.content.contains("-5"));
        assert!(styled_span.is_some());
        let style = styled_span.unwrap().style;
        // Should have both bold AND red
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_resolve_style_tag_palette_custom_color() {
        let mut palette = HashMap::new();
        palette.insert("danger".to_string(), ColorValue::new("#FF5555"));

        let tag = StyleTag::Color("danger".to_string());
        let style = resolve_style_tag(&tag, &palette);
        assert_eq!(style.fg, Some(Color::Rgb(255, 85, 85)));
    }

    #[test]
    fn test_resolve_style_tag_palette_override_builtin() {
        let mut palette = HashMap::new();
        palette.insert("red".to_string(), ColorValue::new("#FF6666"));

        let tag = StyleTag::Color("red".to_string());
        let style = resolve_style_tag(&tag, &palette);
        assert_eq!(style.fg, Some(Color::Rgb(255, 102, 102)));
    }

    #[test]
    fn test_resolve_style_tag_fallback_to_builtin() {
        let palette = HashMap::new();

        let tag = StyleTag::Color("green".to_string());
        let style = resolve_style_tag(&tag, &palette);
        assert_eq!(style.fg, Some(Color::Green));
    }

    #[test]
    fn test_resolve_style_tag_bold_ignores_palette() {
        let palette = HashMap::new();
        let tag = StyleTag::Bold;
        let style = resolve_style_tag(&tag, &palette);
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_build_styled_spans_with_palette() {
        let mut palette = HashMap::new();
        palette.insert("alert".to_string(), ColorValue::new("#FFAA00"));

        let segments = vec![
            ExpandedSegment {
                kind: ExpandedKind::StyleStart(StyleTag::Color("alert".to_string())),
                text: String::new(),
            },
            ExpandedSegment {
                kind: ExpandedKind::Text,
                text: "warning".to_string(),
            },
            ExpandedSegment {
                kind: ExpandedKind::StyleEnd,
                text: String::new(),
            },
        ];

        let base_style = Style::default().fg(Color::White);
        let spans = build_styled_spans(&segments, base_style, &palette);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(Color::Rgb(255, 170, 0)));
    }

    #[test]
    fn test_resolve_style_tag_unknown_name_falls_back() {
        let palette = HashMap::new();
        let tag = StyleTag::Color("nonexistent".to_string());
        let style = resolve_style_tag(&tag, &palette);
        // Unknown name falls back to Color::Reset via ColorValue
        assert_eq!(style.fg, Some(Color::Reset));
    }
}
