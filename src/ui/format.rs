/// A parsed segment of a format string
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatSegment {
    /// A token like %s, %f, %+, %-, etc.
    Token(char),
    /// Literal text between tokens
    Literal(String),
}

/// Parse a vim-style format string into segments
pub fn parse_format(fmt: &str) -> Vec<FormatSegment> {
    let mut segments = Vec::new();
    let mut chars = fmt.chars().peekable();
    let mut literal = String::new();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek() {
                Some('%') => {
                    chars.next();
                    literal.push('%');
                }
                Some(&token) => {
                    if !literal.is_empty() {
                        segments.push(FormatSegment::Literal(literal.clone()));
                        literal.clear();
                    }
                    chars.next();
                    segments.push(FormatSegment::Token(token));
                }
                None => {
                    literal.push('%');
                }
            }
        } else {
            literal.push(ch);
        }
    }

    if !literal.is_empty() {
        segments.push(FormatSegment::Literal(literal));
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_default_format() {
        let segments = parse_format("%s %f %- %+");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Token('s'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('f'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('-'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('+'),
            ]
        );
    }

    #[test]
    fn test_parse_literal_only() {
        let segments = parse_format("hello world");
        assert_eq!(
            segments,
            vec![FormatSegment::Literal("hello world".to_string())]
        );
    }

    #[test]
    fn test_parse_escaped_percent() {
        let segments = parse_format("%%done");
        assert_eq!(
            segments,
            vec![FormatSegment::Literal("%done".to_string())]
        );
    }

    #[test]
    fn test_parse_adjacent_tokens() {
        let segments = parse_format("%s%f");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Token('s'),
                FormatSegment::Token('f'),
            ]
        );
    }

    #[test]
    fn test_parse_tokens_with_surrounding_literal() {
        let segments = parse_format("[%s] %f | %- %+");
        assert_eq!(
            segments,
            vec![
                FormatSegment::Literal("[".to_string()),
                FormatSegment::Token('s'),
                FormatSegment::Literal("] ".to_string()),
                FormatSegment::Token('f'),
                FormatSegment::Literal(" | ".to_string()),
                FormatSegment::Token('-'),
                FormatSegment::Literal(" ".to_string()),
                FormatSegment::Token('+'),
            ]
        );
    }

    #[test]
    fn test_parse_empty_string() {
        let segments = parse_format("");
        assert!(segments.is_empty());
    }
}
