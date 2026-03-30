use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::api::{AnalysisDetails, AnalysisInfo};
use crate::tui::app::App;

const COLOR_GOOD: Color = Color::Rgb(90, 220, 140);
const COLOR_BAD: Color = Color::Rgb(255, 95, 95);
const COLOR_WARN: Color = Color::Rgb(255, 205, 90);
const COLOR_MUTED: Color = Color::Rgb(120, 120, 135);

pub fn draw(f: &mut Frame, app: &App, details: Option<&AnalysisDetails>, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(5),    // content
        Constraint::Length(1), // help
    ])
    .split(area);

    draw_title(f, app, details, chunks[0]);
    draw_content(f, app, details, chunks[1]);
    draw_help(f, chunks[2]);
}

fn draw_title(f: &mut Frame, app: &App, details: Option<&AnalysisDetails>, area: Rect) {
    let (pkg, ver, status) = if let Some(d) = details {
        (d.package.as_str(), d.version.as_str(), d.status.as_str())
    } else if let Some(idx) = app.selected_package_index() {
        let p = &app.packages[idx];
        (p.name.as_str(), p.version.as_str(), p.status.as_str())
    } else {
        ("?", "?", "?")
    };

    let (icon, style) = status_icon_style(status);

    let line = Line::from(vec![
        Span::styled(" ← ", Style::default().fg(Color::DarkGray)),
        Span::styled(pkg, Style::default().fg(Color::Cyan).bold()),
        Span::raw(" "),
        Span::styled(ver, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(format!("{icon} {status}"), style),
    ]);

    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
        area,
    );
}

fn draw_content(f: &mut Frame, app: &App, details: Option<&AnalysisDetails>, area: Rect) {
    if app.detail_loading {
        let spinner = app.spinner_char();
        let line = Line::from(vec![Span::styled(
            format!("  {spinner} Loading analysis details..."),
            Style::default().fg(COLOR_WARN),
        )]);
        f.render_widget(Paragraph::new(vec![Line::raw(""), line]), area);
        return;
    }

    let Some(details) = details else {
        let line = Line::from(Span::styled(
            "  No details available.",
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(Paragraph::new(vec![Line::raw(""), line]), area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw(""));

    // Status
    let (icon, style) = status_icon_style(&details.status);
    lines.push(Line::from(vec![
        Span::styled("  Status:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{icon} {}", details.status), style),
    ]));

    if let Some(analysis) = &details.analysis {
        render_analysis(&mut lines, analysis, &details.status);
    } else {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  Analysis not yet available. Check back later.",
            Style::default().fg(COLOR_WARN),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false })
        .scroll((app.scroll_offset, 0));

    f.render_widget(paragraph, area);
}

fn render_analysis<'a>(lines: &mut Vec<Line<'a>>, a: &'a AnalysisInfo, status: &str) {
    let is_bad = matches!(status, "rejected" | "failed");

    // Risk score with bar
    if let Some(risk) = a.risk_score {
        let bar = render_risk_bar(risk);
        let level = if risk < 0.3 {
            "LOW"
        } else if risk < 0.6 {
            "MEDIUM"
        } else {
            "HIGH"
        };
        let level_color = if is_bad || risk >= 0.6 {
            COLOR_BAD
        } else if risk < 0.3 {
            COLOR_GOOD
        } else {
            COLOR_WARN
        };

        lines.push(Line::from(vec![
            Span::styled("  Risk:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{risk:.2}"),
                Style::default().fg(level_color).bold(),
            ),
            Span::styled(" / 1.0  ", Style::default().fg(Color::DarkGray)),
            Span::styled(bar, Style::default().fg(level_color)),
            Span::raw("  "),
            Span::styled(level, Style::default().fg(level_color).bold()),
        ]));
    }

    // Type
    lines.push(Line::from(vec![
        Span::styled("  Type:      ", Style::default().fg(Color::DarkGray)),
        Span::styled(&a.analysis_type, Style::default().fg(Color::White)),
    ]));

    // Model
    if let Some(model) = &a.model_used {
        let mut spans = vec![
            Span::styled("  Model:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(model, Style::default().fg(Color::White)),
        ];
        if let (Some(p), Some(c)) = (a.prompt_tokens, a.completion_tokens) {
            spans.push(Span::styled(
                format!(" ({p} prompt + {c} completion tokens)"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Date
    lines.push(Line::from(vec![
        Span::styled("  Analyzed:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(&a.analyzed_at, Style::default().fg(Color::White)),
    ]));

    // Reasoning
    if let Some(reasoning) = &a.reasoning {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  Reasoning",
            Style::default().fg(Color::Cyan).bold(),
        )));
        lines.push(Line::from(Span::styled(
            "  ─────────────────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        )));
        for text_line in reasoning.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {text_line}"),
                Style::default().fg(Color::White),
            )));
        }
    }

    // Findings from llm_result
    if let Some(flags) = a
        .llm_result
        .as_ref()
        .and_then(|llm| llm.get("flags"))
        .and_then(|f| f.as_array())
        .filter(|flags| !flags.is_empty())
    {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("  Findings ({})", flags.len()),
            Style::default().fg(Color::Cyan).bold(),
        )));
        lines.push(Line::from(Span::styled(
            "  ─────────────────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        )));

        for flag in flags {
            let severity = flag
                .get("severity")
                .and_then(|s| s.as_str())
                .unwrap_or("info");
            let file = flag
                .get("file_path")
                .and_then(|s| s.as_str())
                .unwrap_or("?");
            let line_range = flag
                .get("line_range")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let desc = flag
                .get("description")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let confidence = flag.get("confidence").and_then(|c| c.as_f64());

            let sev_color = severity_color(severity);
            let location = if line_range.is_empty() {
                file.to_string()
            } else {
                format!("{file}:{line_range}")
            };

            let mut spans = vec![
                Span::raw("  "),
                Span::styled(
                    format!("[{severity}]"),
                    Style::default().fg(sev_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(location, Style::default().fg(Color::White)),
            ];

            if let Some(c) = confidence {
                spans.push(Span::styled(
                    format!("  (confidence: {c:.1})"),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            let desc_color = match severity {
                "critical" | "high" => COLOR_BAD,
                "medium" => COLOR_WARN,
                _ => COLOR_MUTED,
            };

            lines.push(Line::from(spans));
            lines.push(Line::from(Span::styled(
                format!("    {desc}"),
                Style::default().fg(desc_color),
            )));
            lines.push(Line::raw(""));
        }
    }

    // Diff summary
    if let Some(diff) = &a.diff_summary {
        if let Some(summary) = diff
            .as_str()
            .or_else(|| diff.get("summary").and_then(|s| s.as_str()))
        {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  Diff Summary",
                Style::default().fg(Color::Cyan).bold(),
            )));
            lines.push(Line::from(Span::styled(
                "  ─────────────────────────────────────────────────",
                Style::default().fg(Color::DarkGray),
            )));
            for text_line in summary.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {text_line}"),
                    Style::default().fg(Color::White),
                )));
            }
        }
    }
}

fn render_risk_bar(score: f32) -> String {
    let filled = (score * 10.0).round() as usize;
    let empty = 10usize.saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn severity_color(severity: &str) -> Color {
    match severity {
        "critical" => COLOR_BAD,
        "high" => Color::Rgb(255, 140, 140),
        "medium" => COLOR_WARN,
        "low" => Color::Cyan,
        _ => COLOR_MUTED,
    }
}

fn status_icon_style(status: &str) -> (String, Style) {
    match status {
        "approved" => ("✓".to_string(), Style::default().fg(COLOR_GOOD)),
        "rejected" => ("✗".to_string(), Style::default().fg(COLOR_BAD)),
        "failed" => ("✗".to_string(), Style::default().fg(COLOR_BAD)),
        "needs_review" => ("?".to_string(), Style::default().fg(COLOR_WARN)),
        "pending" | "analyzing" => ("◌".to_string(), Style::default().fg(COLOR_WARN)),
        _ => (" ".to_string(), Style::default()),
    }
}

fn draw_help(f: &mut Frame, area: Rect) {
    let key_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(180, 180, 180))
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::Rgb(160, 160, 160));
    let bar_style = Style::default()
        .bg(Color::Rgb(25, 25, 30))
        .fg(Color::Rgb(160, 160, 160));

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(" Esc ", key_style),
        Span::styled(" back  ", label_style),
        Span::styled(" j/k ", key_style),
        Span::styled(" scroll  ", label_style),
        Span::styled(" g/G ", key_style),
        Span::styled(" top/bottom  ", label_style),
        Span::styled(" q ", key_style),
        Span::styled(" quit", label_style),
    ]);

    f.render_widget(Paragraph::new(line).style(bar_style), area);
}
