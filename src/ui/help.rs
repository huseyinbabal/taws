use crate::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 70, f.area());

    f.render_widget(Clear, area);

    let mut help_text: Vec<Line<'static>> = vec![
        Line::from(""),
        create_section("Navigation"),
        create_key_line("j / ↓", "Move down"),
        create_key_line("k / ↑", "Move up"),
        create_key_line("gg / Home", "Go to top"),
        create_key_line("G / End", "Go to bottom"),
        create_key_line("PgUp / Ctrl+b", "Page up"),
        create_key_line("PgDn / Ctrl+f", "Page down"),
        create_key_line("]", "Next page (load more)"),
        create_key_line("[", "Previous page"),
        create_key_line("R", "Refresh list"),
        Line::from(""),
        create_section("Views"),
        create_key_line("d / Enter", "Show details panel"),
        create_key_line("J", "Show JSON view"),
        create_key_line("?", "Toggle help"),
        Line::from(""),
        create_section("Details View"),
        create_key_line("j / k", "Scroll up/down"),
        create_key_line("PgUp / Ctrl+b", "Page up"),
        create_key_line("PgDn / Ctrl+f", "Page down"),
        create_key_line("g / Home", "Go to top"),
        create_key_line("G / End", "Go to bottom"),
        create_key_line("/", "Search in content"),
        create_key_line("n / N", "Next/prev match"),
        create_key_line("q / Esc / d", "Close details"),
        Line::from(""),
    ];

    // Add resource-specific actions section
    if let Some(resource) = app.current_resource() {
        if !resource.actions.is_empty() {
            let section_title = format!("{} Actions", resource.display_name);
            help_text.push(create_section(&section_title));
            for action in &resource.actions {
                let shortcut = action.shortcut.as_deref().unwrap_or(&action.key);
                help_text.push(create_key_line(shortcut, &action.display_name));
            }
            help_text.push(Line::from(""));
        }

        // Add sub-resources navigation section if resource has sub-resources
        if !resource.sub_resources.is_empty() {
            help_text.push(create_section("Sub-resources"));
            for sub in &resource.sub_resources {
                help_text.push(create_key_line(&sub.shortcut, &sub.display_name));
            }
            help_text.push(Line::from(""));
        }
    }

    // Add Log Tail section only for CloudWatch log streams
    if app.current_resource_key == "cloudwatch-log-streams" {
        help_text.extend(vec![
            create_section("Log Tail Mode"),
            create_key_line("t", "Tail logs"),
            create_key_line("j / k", "Scroll up/down"),
            create_key_line("G", "Go to bottom (live mode)"),
            create_key_line("g", "Go to top"),
            create_key_line("SPACE", "Pause/resume"),
            create_key_line("q / Esc", "Exit log tail"),
            Line::from(""),
        ]);
    }

    // Add remaining static sections
    help_text.extend(vec![
        create_section("Modes"),
        create_key_line("/", "Filter mode"),
        create_key_line(":", "Command mode"),
        Line::from(""),
        create_section("Common Commands"),
        create_key_line(":profiles", "Switch AWS profile"),
        create_key_line(":regions", "Switch AWS region"),
        create_key_line("Backspace", "Go back (parent resource)"),
        Line::from(""),
        create_key_line("Esc", "Close / Cancel"),
        create_key_line("Ctrl+c", "Quit application"),
    ]);

    let block = Block::default()
        .title(" Help ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text).block(block);

    f.render_widget(paragraph, area);
}

fn create_section(title: &str) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!("  {} ", title),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )])
}

fn create_key_line(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("{:>15}", key),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(description.to_string(), Style::default().fg(Color::White)),
    ])
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
