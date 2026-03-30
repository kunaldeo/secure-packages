use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::tui::app::App;

const COLOR_GOOD: Color = Color::Rgb(90, 220, 140);
const COLOR_BAD: Color = Color::Rgb(255, 95, 95);
const COLOR_WARN: Color = Color::Rgb(255, 205, 90);
const COLOR_SURFACE: Color = Color::Rgb(30, 30, 30);
const COLOR_SELECTED: Color = Color::Rgb(40, 40, 50);

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(5),    // table
        Constraint::Length(1), // summary
        Constraint::Length(1), // help / filter
    ])
    .split(area);

    draw_title(f, app, chunks[0]);
    draw_table(f, app, chunks[1]);
    draw_summary(f, app, chunks[2]);
    draw_help(f, app, chunks[3]);
}

fn draw_title(f: &mut Frame, app: &App, area: Rect) {
    let elapsed = format!("{:.1}s", app.elapsed_secs());
    let status_indicator = if app.all_resolved {
        Span::styled(" done ", Style::default().fg(COLOR_GOOD).bold())
    } else {
        Span::styled(
            format!(" {} ", app.spinner_char()),
            Style::default().fg(COLOR_WARN).bold(),
        )
    };

    let line = Line::from(vec![
        Span::styled(" sp-client ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("── "),
        Span::styled(&app.requirements_file, Style::default().fg(Color::White)),
        Span::raw(" ── "),
        Span::styled(
            format!("{} packages", app.packages.len()),
            Style::default().fg(Color::White),
        ),
        Span::raw(" ── "),
        Span::styled(elapsed, Style::default().fg(Color::DarkGray)),
        status_indicator,
    ]);

    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
        area,
    );
}

fn draw_table(f: &mut Frame, app: &App, area: Rect) {
    let filtered = app.filtered_indices();

    let header = Row::new(vec![
        Cell::from("Package").style(Style::default().bold().fg(Color::Cyan)),
        Cell::from("Version").style(Style::default().bold().fg(Color::Cyan)),
        Cell::from("Status").style(Style::default().bold().fg(Color::Cyan)),
        Cell::from("Risk").style(Style::default().bold().fg(Color::Cyan)),
        Cell::from("Summary").style(Style::default().bold().fg(Color::Cyan)),
    ])
    .height(1)
    .bottom_margin(0);

    let rows: Vec<Row> = filtered
        .iter()
        .map(|&i| {
            let p = &app.packages[i];
            let (icon, status_style) = status_display(p, app);
            let risk = p
                .risk_score
                .map(|r| format!("{:.2}", r))
                .unwrap_or_else(|| "  -".to_string());
            let is_bad = matches!(p.status.as_str(), "rejected" | "failed");
            let risk_style = if is_bad {
                Style::default().fg(COLOR_BAD)
            } else {
                p.risk_score
                    .map(|r| risk_color(r))
                    .unwrap_or(Style::default().fg(Color::DarkGray))
            };
            let summary = truncate(
                if p.status == "failed" {
                    p.error.as_deref().unwrap_or("")
                } else {
                    p.reasoning.as_deref().unwrap_or("")
                },
                50,
            );

            let row_color = if is_bad { COLOR_BAD } else { Color::White };
            let dim_color = if is_bad { COLOR_BAD } else { Color::DarkGray };

            Row::new(vec![
                Cell::from(Span::styled(p.name.clone(), Style::default().fg(row_color))),
                Cell::from(Span::styled(
                    p.version.clone(),
                    Style::default().fg(dim_color),
                )),
                Cell::from(Span::styled(format!("{icon} {}", p.status), status_style)),
                Cell::from(Span::styled(risk, risk_style)),
                Cell::from(Span::styled(summary, Style::default().fg(dim_color))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(28),
        Constraint::Percentage(12),
        Constraint::Percentage(16),
        Constraint::Percentage(8),
        Constraint::Percentage(36),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::NONE))
        .row_highlight_style(
            Style::default()
                .bg(COLOR_SELECTED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = TableState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_summary(f: &mut Frame, app: &App, area: Rect) {
    let approved = app
        .packages
        .iter()
        .filter(|p| p.status == "approved")
        .count();
    let rejected = app
        .packages
        .iter()
        .filter(|p| p.status == "rejected" || p.status == "failed")
        .count();
    let pending = app
        .packages
        .iter()
        .filter(|p| p.status == "pending" || p.status == "analyzing")
        .count();
    let review = app
        .packages
        .iter()
        .filter(|p| p.status == "needs_review")
        .count();

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{approved} approved"),
            Style::default().fg(COLOR_GOOD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("{rejected} rejected"),
            Style::default().fg(COLOR_BAD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("{pending} pending"),
            Style::default().fg(COLOR_WARN),
        ),
    ];

    if review > 0 {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            format!("{review} needs review"),
            Style::default().fg(COLOR_WARN),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(COLOR_SURFACE)),
        area,
    );
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let key_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(180, 180, 180))
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::Rgb(160, 160, 160));
    let bar_style = Style::default()
        .bg(Color::Rgb(25, 25, 30))
        .fg(Color::Rgb(160, 160, 160));

    let line = if app.filter.active {
        Line::from(vec![
            Span::styled(" / ", key_style),
            Span::styled(&app.filter.query, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(COLOR_WARN)),
            Span::raw("   "),
            Span::styled(" Esc ", key_style),
            Span::styled(" clear  ", label_style),
            Span::styled(" Enter ", key_style),
            Span::styled(" confirm", label_style),
        ])
    } else {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(" j/k ", key_style),
            Span::styled(" navigate  ", label_style),
            Span::styled(" Enter ", key_style),
            Span::styled(" details  ", label_style),
            Span::styled(" / ", key_style),
            Span::styled(" filter  ", label_style),
            Span::styled(" g/G ", key_style),
            Span::styled(" top/bottom  ", label_style),
            Span::styled(" q ", key_style),
            Span::styled(" quit", label_style),
        ])
    };

    f.render_widget(Paragraph::new(line).style(bar_style), area);
}

fn status_display(p: &crate::api::PackageStatus, app: &App) -> (String, Style) {
    match p.status.as_str() {
        "approved" => ("✓".to_string(), Style::default().fg(COLOR_GOOD)),
        "rejected" => ("✗".to_string(), Style::default().fg(COLOR_BAD)),
        "failed" => ("✗".to_string(), Style::default().fg(COLOR_BAD)),
        "needs_review" => ("?".to_string(), Style::default().fg(COLOR_WARN)),
        "pending" | "analyzing" => (
            app.spinner_char().to_string(),
            Style::default().fg(COLOR_WARN),
        ),
        _ => (" ".to_string(), Style::default()),
    }
}

fn risk_color(score: f32) -> Style {
    if score < 0.3 {
        Style::default().fg(COLOR_GOOD)
    } else if score < 0.6 {
        Style::default().fg(COLOR_WARN)
    } else {
        Style::default().fg(COLOR_BAD)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max - 1])
    } else {
        s.to_string()
    }
}
