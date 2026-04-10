//! Parse color value strings from theme files into ratatui Colors.

use ratatui::style::Color;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ColorParseError {
    #[error("invalid hex color: {0}")]
    InvalidHex(String),
    #[error("unknown named color: {0}")]
    UnknownName(String),
    #[error("indexed color out of range (0-255): {0}")]
    IndexOutOfRange(String),
    #[error("invalid color format: {0}")]
    Invalid(String),
}

/// Parse a color value string into a ratatui `Color`.
///
/// Supported formats:
/// - Hex: `"#RRGGBB"` or `"#RGB"` (3-digit shorthand)
/// - Named: `"red"`, `"blue"`, `"lightgreen"`, etc. (case-insensitive)
/// - Indexed: `"256:N"` where N is in 0..=255
pub fn parse_color(s: &str) -> Result<Color, ColorParseError> {
    let s = s.trim();

    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }

    if let Some(idx) = s.strip_prefix("256:") {
        return parse_indexed(idx);
    }

    parse_named(s)
}

fn parse_hex(hex: &str) -> Result<Color, ColorParseError> {
    let expanded = if hex.len() == 3 {
        let mut out = String::with_capacity(6);
        for c in hex.chars() {
            out.push(c);
            out.push(c);
        }
        out
    } else {
        hex.to_string()
    };

    if expanded.len() != 6 {
        return Err(ColorParseError::InvalidHex(format!("#{hex}")));
    }

    let r = u8::from_str_radix(&expanded[0..2], 16)
        .map_err(|_| ColorParseError::InvalidHex(format!("#{hex}")))?;
    let g = u8::from_str_radix(&expanded[2..4], 16)
        .map_err(|_| ColorParseError::InvalidHex(format!("#{hex}")))?;
    let b = u8::from_str_radix(&expanded[4..6], 16)
        .map_err(|_| ColorParseError::InvalidHex(format!("#{hex}")))?;

    Ok(Color::Rgb(r, g, b))
}

fn parse_indexed(s: &str) -> Result<Color, ColorParseError> {
    let n: u16 = s
        .parse()
        .map_err(|_| ColorParseError::Invalid(format!("256:{s}")))?;
    if n > 255 {
        return Err(ColorParseError::IndexOutOfRange(format!("256:{s}")));
    }
    Ok(Color::Indexed(n as u8))
}

fn parse_named(s: &str) -> Result<Color, ColorParseError> {
    match s.to_lowercase().as_str() {
        "black" => Ok(Color::Black),
        "red" => Ok(Color::Red),
        "green" => Ok(Color::Green),
        "yellow" => Ok(Color::Yellow),
        "blue" => Ok(Color::Blue),
        "magenta" => Ok(Color::Magenta),
        "cyan" => Ok(Color::Cyan),
        "gray" | "grey" => Ok(Color::Gray),
        "darkgray" | "darkgrey" | "dark_gray" | "dark_grey" => Ok(Color::DarkGray),
        "lightred" | "light_red" => Ok(Color::LightRed),
        "lightgreen" | "light_green" => Ok(Color::LightGreen),
        "lightyellow" | "light_yellow" => Ok(Color::LightYellow),
        "lightblue" | "light_blue" => Ok(Color::LightBlue),
        "lightmagenta" | "light_magenta" => Ok(Color::LightMagenta),
        "lightcyan" | "light_cyan" => Ok(Color::LightCyan),
        "white" => Ok(Color::White),
        _ => Err(ColorParseError::UnknownName(s.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_6digit() {
        assert_eq!(parse_color("#cdd6f4").unwrap(), Color::Rgb(205, 214, 244));
    }

    #[test]
    fn test_parse_hex_6digit_uppercase() {
        assert_eq!(parse_color("#CDD6F4").unwrap(), Color::Rgb(205, 214, 244));
    }

    #[test]
    fn test_parse_hex_3digit() {
        assert_eq!(parse_color("#fff").unwrap(), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_parse_hex_3digit_mixed() {
        assert_eq!(parse_color("#a1b").unwrap(), Color::Rgb(170, 17, 187));
    }

    #[test]
    fn test_parse_hex_invalid_length() {
        assert!(matches!(
            parse_color("#fffff"),
            Err(ColorParseError::InvalidHex(_))
        ));
    }

    #[test]
    fn test_parse_hex_invalid_chars() {
        assert!(matches!(
            parse_color("#zzzzzz"),
            Err(ColorParseError::InvalidHex(_))
        ));
    }

    #[test]
    fn test_parse_named_basic() {
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("green").unwrap(), Color::Green);
        assert_eq!(parse_color("blue").unwrap(), Color::Blue);
    }

    #[test]
    fn test_parse_named_case_insensitive() {
        assert_eq!(parse_color("Red").unwrap(), Color::Red);
        assert_eq!(parse_color("RED").unwrap(), Color::Red);
    }

    #[test]
    fn test_parse_named_light_variants() {
        assert_eq!(parse_color("lightred").unwrap(), Color::LightRed);
        assert_eq!(parse_color("light_red").unwrap(), Color::LightRed);
    }

    #[test]
    fn test_parse_named_gray_variants() {
        assert_eq!(parse_color("gray").unwrap(), Color::Gray);
        assert_eq!(parse_color("grey").unwrap(), Color::Gray);
        assert_eq!(parse_color("darkgray").unwrap(), Color::DarkGray);
        assert_eq!(parse_color("dark_grey").unwrap(), Color::DarkGray);
    }

    #[test]
    fn test_parse_named_unknown() {
        assert!(matches!(
            parse_color("notacolor"),
            Err(ColorParseError::UnknownName(_))
        ));
    }

    #[test]
    fn test_parse_indexed_valid() {
        assert_eq!(parse_color("256:0").unwrap(), Color::Indexed(0));
        assert_eq!(parse_color("256:196").unwrap(), Color::Indexed(196));
        assert_eq!(parse_color("256:255").unwrap(), Color::Indexed(255));
    }

    #[test]
    fn test_parse_indexed_out_of_range() {
        assert!(matches!(
            parse_color("256:256"),
            Err(ColorParseError::IndexOutOfRange(_))
        ));
        assert!(matches!(
            parse_color("256:9999"),
            Err(ColorParseError::IndexOutOfRange(_))
        ));
    }

    #[test]
    fn test_parse_indexed_invalid() {
        assert!(matches!(
            parse_color("256:abc"),
            Err(ColorParseError::Invalid(_))
        ));
    }

    #[test]
    fn test_parse_with_whitespace() {
        assert_eq!(parse_color("  red  ").unwrap(), Color::Red);
        assert_eq!(parse_color("  #fff  ").unwrap(), Color::Rgb(255, 255, 255));
    }
}
