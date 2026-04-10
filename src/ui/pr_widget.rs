//! PR pane rendering.
//!
//! Renders pull-request information (status, reviews, checks, comments,
//! mergeable state) inside a bordered pane whose border colour matches the
//! GitHub PR state colour conventions.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::state::{CheckStatus, MergeableStatus, PrDisplayInfo, PrState, PrStatus, ReviewState};
use crate::theme::Theme;

/// Border colour matching GitHub conventions for PR state.
fn pr_border_color(status: &PrStatus) -> Color {
    match status {
        PrStatus::Open => Color::Green,
        PrStatus::Closed => Color::Red,
        PrStatus::Merged => Color::Magenta,
        PrStatus::Draft => Color::Gray,
    }
}

/// Render the PR widget into `area`.
///
/// If the PR state has no data (and is not loading/errored), this renders
/// nothing. Otherwise it draws a bordered pane with PR details.
pub fn render_pr_widget(
    frame: &mut Frame,
    pr_state: &PrState,
    show_labels: bool,
    theme: &Theme,
    area: Rect,
) {
    if pr_state.loading {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(" PR ")
            .title_style(Style::default().fg(theme.header_text));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new("Loading PR info...").style(Style::default().fg(theme.empty_text));
        frame.render_widget(msg, inner);
        return;
    }

    if let Some(ref error) = pr_state.error {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Red))
            .title(" PR ")
            .title_style(Style::default().fg(theme.header_text));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new(format!("Error: {error}")).style(Style::default().fg(Color::Red));
        frame.render_widget(msg, inner);
        return;
    }

    let info = match &pr_state.info {
        Some(info) => info,
        None => return, // Nothing to show
    };

    render_pr_info(frame, info, show_labels, theme, area);
}

/// Render actual PR data.
fn render_pr_info(
    frame: &mut Frame,
    info: &PrDisplayInfo,
    show_labels: bool,
    theme: &Theme,
    area: Rect,
) {
    let border_color = pr_border_color(&info.state);

    let title = format!(" PR #{} ", info.number);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(theme.header_text));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        &info.title,
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());

    // Status
    let status_str = match info.state {
        PrStatus::Open => "Open",
        PrStatus::Closed => "Closed",
        PrStatus::Merged => "Merged",
        PrStatus::Draft => "Draft",
    };
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().fg(theme.header_separator)),
        Span::styled(status_str, Style::default().fg(border_color)),
    ]));

    // Mergeable
    let (merge_text, merge_color) = match info.mergeable {
        MergeableStatus::Clean => ("Clean", Color::Green),
        MergeableStatus::Conflicts => ("Conflicts", Color::Red),
        MergeableStatus::Behind => ("Behind", Color::Yellow),
        MergeableStatus::Unknown => ("Unknown", Color::Gray),
    };
    lines.push(Line::from(vec![
        Span::styled("Mergeable: ", Style::default().fg(theme.header_separator)),
        Span::styled(merge_text, Style::default().fg(merge_color)),
    ]));

    // Comments
    lines.push(Line::from(vec![
        Span::styled("Comments: ", Style::default().fg(theme.header_separator)),
        Span::styled(
            info.comment_count.to_string(),
            Style::default().fg(theme.fg),
        ),
    ]));

    // Checks
    let checks = &info.checks;
    let checks_summary = format!(
        "{}/{} passed, {} failed, {} pending",
        checks.passed, checks.total, checks.failed, checks.pending
    );
    lines.push(Line::from(vec![
        Span::styled("Checks: ", Style::default().fg(theme.header_separator)),
        Span::styled(checks_summary, Style::default().fg(theme.fg)),
    ]));

    // Individual checks (show up to 5)
    for check in checks.checks.iter().take(5) {
        let (icon, color) = match check.status {
            CheckStatus::Passed => ("  +", Color::Green),
            CheckStatus::Failed => ("  x", Color::Red),
            CheckStatus::Pending => ("  o", Color::Yellow),
            CheckStatus::Running => ("  ~", Color::Cyan),
        };
        lines.push(Line::from(vec![
            Span::styled(icon, Style::default().fg(color)),
            Span::styled(format!(" {}", check.name), Style::default().fg(theme.fg)),
        ]));
    }

    // Reviews
    if !info.reviews.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Reviews:",
            Style::default().fg(theme.header_separator),
        )));

        for review in &info.reviews {
            let (icon, color) = match review.state {
                ReviewState::Approved => ("+", Color::Green),
                ReviewState::ChangesRequested => ("!", Color::Red),
                ReviewState::Pending => ("o", Color::Yellow),
                ReviewState::Commented => (".", Color::Cyan),
                ReviewState::Dismissed => ("-", Color::Gray),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {icon}"), Style::default().fg(color)),
                Span::styled(
                    format!(" {}", review.reviewer),
                    Style::default().fg(theme.fg),
                ),
            ]));
        }
    }

    // Assignees
    if !info.assignees.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("Assignees: ", Style::default().fg(theme.header_separator)),
            Span::styled(info.assignees.join(", "), Style::default().fg(theme.fg)),
        ]));
    }

    // Labels
    if show_labels && !info.labels.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Labels: ", Style::default().fg(theme.header_separator)),
            Span::styled(info.labels.join(", "), Style::default().fg(theme.fg)),
        ]));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}
