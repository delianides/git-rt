//! Full-screen help overlay listing built-in keybindings.
//!
//! Shown when the user presses `?`. Displays a centred panel at ~85% of
//! the terminal area with a two-column list: key on the left, description
//! on the right. Dismissible with `?`, `Esc`, `q`, or `Space`.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::colors;
use crate::ui::fit;

/// Return a centred `Rect` that occupies `percent_x`% width and `percent_y`%
/// height of `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    let h = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Build the list of help lines (section headers + key/description rows).
pub fn build_help_lines() -> Vec<Line<'static>> {
    let key_style = Style::default()
        .fg(colors::INSERTIONS)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(colors::HEADER_TEXT);
    let header_style = Style::default()
        .fg(colors::HEADER_TEXT)
        .add_modifier(Modifier::BOLD);

    let entries: &[(&str, &[(&str, &str)])] = &[
        (
            "Navigation",
            &[
                ("j / ↓", "Select next file"),
                ("k / ↑", "Select previous file"),
                ("gg / G", "Jump to top / bottom"),
                ("Ctrl+u / Ctrl+d", "Half-page up / down"),
                ("Ctrl+b / Ctrl+f", "Full-page up / down"),
            ],
        ),
        (
            "Diff",
            &[
                ("Enter", "Open diff"),
                ("d", "Toggle diff"),
                ("Space", "Toggle diff"),
                ("l / →", "Open diff"),
                ("Enter / l / →", "Tree: toggle dir, File: open diff"),
                ("Enter / Space", "Normal: collapse/expand group header"),
                ("h / ←", "Close diff"),
                ("j / k", "Scroll diff (inside modal)"),
                ("gg / G / Ctrl+u/d/f/b", "Jump & page within diff"),
            ],
        ),
        (
            "Other",
            &[
                ("m", "Cycle view mode"),
                ("r", "Refresh"),
                ("s", "Switch worktree"),
                ("e", "Edit selected file"),
                ("p", "Open PR in browser"),
                ("?", "Toggle this help"),
                ("q / Esc", "Dismiss overlay / quit"),
                ("Ctrl+C", "Quit"),
            ],
        ),
    ];

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(""));

    // Width of the widest key across every section, plus a 2-column gap, so
    // all descriptions line up regardless of key length. Measured by terminal
    // display width (not char count) to stay correct with arrow glyphs.
    let key_w = entries
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(key, _)| fit::display_width(key))
        .max()
        .unwrap_or(0)
        + 2;

    for (section, rows) in entries {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(section.to_string(), header_style),
        ]));
        for (key, desc) in *rows {
            let pad = key_w.saturating_sub(fit::display_width(key));
            let key_padded = format!("{key}{}", " ".repeat(pad));
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(key_padded, key_style),
                Span::styled(desc.to_string(), desc_style),
            ]));
        }
        lines.push(Line::from(""));
    }

    lines
}

/// Render the help overlay onto `frame`.
pub fn render_help_overlay(frame: &mut Frame) {
    let area = frame.area();
    let overlay_rect = centered_rect(85, 85, area);

    frame.render_widget(Clear, overlay_rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::BORDER_FOCUSED))
        .title(" Keybindings ")
        .title_style(Style::default().fg(colors::HEADER_TEXT));

    let inner = block.inner(overlay_rect);
    frame.render_widget(block, overlay_rect);

    let lines = build_help_lines();
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fit::display_width;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    fn lines_text(lines: &[Line<'_>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    /// Display width of a row's indent + padded-key (i.e. the column where the
    /// description begins). Only meaningful for key/desc rows (3 spans).
    fn desc_column(line: &Line<'_>) -> Option<usize> {
        if line.spans.len() != 3 {
            return None;
        }
        let prefix: String = line.spans[..2].iter().map(|s| s.content.as_ref()).collect();
        Some(display_width(&prefix))
    }

    #[test]
    fn test_help_descriptions_share_one_column() {
        let lines = build_help_lines();
        let columns: Vec<usize> = lines.iter().filter_map(desc_column).collect();
        assert!(columns.len() > 5, "expected several key/desc rows");
        let first = columns[0];
        assert!(
            columns.iter().all(|&c| c == first),
            "all descriptions must start at the same column, got {columns:?}"
        );
    }

    #[test]
    fn test_help_longest_key_keeps_gap() {
        // "gg / G / Ctrl+u/d/f/b" is the widest key; it must still leave a >=2
        // space gap before its description rather than overflowing.
        let lines = build_help_lines();
        let row = lines
            .iter()
            .find(|l| line_text(l).contains("gg / G / Ctrl+u/d/f/b"))
            .expect("missing longest-key row");
        assert_eq!(row.spans.len(), 3);
        let key_span = row.spans[1].content.as_ref();
        let gap = display_width(key_span) - display_width("gg / G / Ctrl+u/d/f/b");
        assert!(gap >= 2, "expected >=2 space gap, got {gap}");
    }

    #[test]
    fn test_help_lines_contain_all_sections() {
        let lines = build_help_lines();
        let text = lines_text(&lines);
        assert!(text.contains("Navigation"), "missing Navigation section");
        assert!(text.contains("Diff"), "missing Diff section");
        assert!(text.contains("Other"), "missing Other section");
    }

    #[test]
    fn test_help_lines_contain_key_entries() {
        let lines = build_help_lines();
        let text = lines_text(&lines);
        assert!(text.contains("j / ↓"), "missing j/down key");
        assert!(text.contains("Enter"), "missing Enter key");
        assert!(text.contains("Space"), "missing Space key");
        assert!(text.contains("d "), "missing d key");
        assert!(text.contains("s "), "missing s key");
        assert!(text.contains("?"), "missing ? key");
        assert!(text.contains("Ctrl+C"), "missing Ctrl+C");
    }

    #[test]
    fn test_help_lines_contain_descriptions() {
        let lines = build_help_lines();
        let text = lines_text(&lines);
        assert!(text.contains("Select next file"));
        assert!(text.contains("Toggle diff"));
        assert!(text.contains("Open diff"));
        assert!(text.contains("Close diff"));
        assert!(text.contains("Show this help") || text.contains("Toggle this help"));
    }

    #[test]
    fn test_help_lines_contain_mode_key() {
        let lines = build_help_lines();
        assert!(lines.iter().any(|line| {
            let text = line_text(line);
            text.starts_with("    m") && text.contains("Cycle view mode")
        }));
    }
}
