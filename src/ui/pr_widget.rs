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

/// Foreground colour for text on the PR state badge background.
/// Ensures readable contrast against the badge bg color.
fn pr_badge_fg(status: &PrStatus) -> Color {
    match status {
        PrStatus::Open => Color::Rgb(20, 20, 20), // dark on green
        PrStatus::Closed => Color::Rgb(255, 255, 255), // white on red
        PrStatus::Merged => Color::Rgb(255, 255, 255), // white on magenta
        PrStatus::Draft => Color::Rgb(20, 20, 20), // dark on gray
    }
}

/// All-caps label for the PR status badge.
fn pr_status_label(status: &PrStatus) -> &'static str {
    match status {
        PrStatus::Open => "OPEN",
        PrStatus::Closed => "CLOSED",
        PrStatus::Merged => "MERGED",
        PrStatus::Draft => "DRAFT",
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
    let badge_fg = pr_badge_fg(&info.state);
    let badge_label = pr_status_label(&info.state);

    // Build title line: " OPEN  PR #142 · title "
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!(" {} ", badge_label),
            Style::default()
                .fg(badge_fg)
                .bg(border_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" PR #{} ", info.number),
            Style::default().fg(border_color),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        &info.title,
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());

    // Conflicts
    let (merge_text, merge_color) = match info.mergeable {
        MergeableStatus::Clean => ("No conflicts", Color::Green),
        MergeableStatus::Conflicts => ("Has conflicts", Color::Red),
        MergeableStatus::Behind => ("Behind base branch", Color::Yellow),
        MergeableStatus::Unknown => ("Checking...", Color::Gray),
    };
    lines.push(Line::from(vec![
        Span::styled("Conflicts: ", Style::default().fg(theme.header_separator)),
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

    // Checks: "passed/total (skipped skipped)"
    let checks = &info.checks;
    let mut check_spans = vec![Span::styled(
        "Checks: ",
        Style::default().fg(theme.header_separator),
    )];

    let summary_color = if checks.failed > 0 {
        Color::Red
    } else if checks.pending > 0 {
        Color::Yellow
    } else {
        Color::Green
    };
    check_spans.push(Span::styled(
        format!("{}/{}", checks.passed, checks.total),
        Style::default().fg(summary_color),
    ));

    if checks.skipped > 0 {
        check_spans.push(Span::styled(
            format!(" ({} skipped)", checks.skipped),
            Style::default().fg(Color::Gray),
        ));
    }

    lines.push(Line::from(check_spans));

    // Only show individual checks that failed
    let failed_checks: Vec<_> = checks
        .checks
        .iter()
        .filter(|c| c.status == CheckStatus::Failed)
        .collect();
    for check in failed_checks.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled("  ✗ ", Style::default().fg(Color::Red)),
            Span::styled(check.name.clone(), Style::default().fg(theme.fg)),
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
                ReviewState::Approved => ("✓", Color::Green),
                ReviewState::ChangesRequested => ("✗", Color::Red),
                ReviewState::Pending => ("●", Color::Yellow),
                ReviewState::Commented => ("◆", Color::Cyan),
                ReviewState::Dismissed => ("–", Color::Gray),
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
