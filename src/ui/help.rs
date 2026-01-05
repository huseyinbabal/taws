use crate::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(f: &mut Frame, _app: &App) {
    let area = centered_rect(60, 70, f.area());

    f.render_widget(Clear, area);

    let help_text = vec![
        Line::from(""),
        create_section("Navigation"),
        create_key_line("j / ↓", "Move down"),
        create_key_line("k / ↑", "Move up"),
        create_key_line("gg / Home", "Go to top"),
        create_key_line("G / End", "Go to bottom"),
        create_key_line("Ctrl+d", "Page down"),
        create_key_line("Ctrl+u", "Page up"),
        create_key_line("]", "Next page (load more)"),
        create_key_line("[", "Previous page"),
        Line::from(""),
        create_section("Views"),
        create_key_line("d / Enter", "Show details panel"),
        create_key_line("J", "Show JSON view"),
        create_key_line("?", "Toggle help"),
        Line::from(""),
        create_section("EC2 Actions"),
        create_key_line("s", "Start instance"),
        create_key_line("S", "Stop instance"),
        create_key_line("Ctrl+d", "Terminate instance"),
        Line::from(""),
        create_section("Log Tail Mode"),
        create_key_line("t", "Tail logs (on log stream)"),
        create_key_line("j / k", "Scroll up/down"),
        create_key_line("G", "Go to bottom (live mode)"),
        create_key_line("g", "Go to top"),
        create_key_line("SPACE", "Pause/resume"),
        create_key_line("q / Esc", "Exit log tail"),
        Line::from(""),
        create_section("Auto-refresh"),
        create_key_line("", "List refreshes every 5s"),
        Line::from(""),
        create_section("Modes"),
        create_key_line("/", "Filter mode"),
        create_key_line(":", "Resources mode"),
        Line::from(""),
        create_section("Resources"),
        create_key_line(":ec2", "EC2 instances view"),
        create_key_line(":vpc", "VPC view"),
        create_key_line(":profiles", "List AWS profiles"),
        create_key_line(":regions", "List AWS regions"),
        Line::from(""),
        create_key_line("Esc", "Close / Cancel"),
        create_key_line("Ctrl+c", "Quit application"),
    ];

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

fn create_section(title: &str) -> Line<'_> {
    Line::from(vec![Span::styled(
        format!("  {} ", title),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )])
}

fn create_key_line<'a>(key: &'a str, description: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("{:>15}", key),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(description, Style::default().fg(Color::White)),
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
