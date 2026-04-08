#[allow(unused_imports)]
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

#[allow(unused_imports)]
use crate::config::ColorValue;
#[allow(unused_imports)]
use crate::state::AppState;

/// A parsed segment of a statusbar format string
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

/// Parse a statusbar format string into segments.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
