use crate::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    let Some(state) = app.region_picker_state() else {
        return;
    };

    let area = centered_rect(60, 70, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Select Region ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    let instructions = Paragraph::new(Line::from(vec![
        Span::styled("ENTER", Style::default().fg(Color::Yellow)),
        Span::raw(" jump  "),
        Span::styled("ESC", Style::default().fg(Color::Yellow)),
        Span::raw(" cancel"),
    ]));
    f.render_widget(instructions, chunks[0]);

    let rows = app.available_regions.iter().map(|region| {
        let style = if region == &app.region {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        Row::new(vec![Cell::from(format!("{}", region)).style(style)])
    });

    let mut table_state = TableState::default();
    if app.available_regions.is_empty() {
        table_state.select(None);
    } else {
        let max_index = app.available_regions.len().saturating_sub(1);
        table_state.select(Some(state.cursor.min(max_index)));
    }

    let table = Table::new(rows, [Constraint::Percentage(100)]).row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(table, chunks[1], &mut table_state);

    let footer = if let Some(region) = app.available_regions.get(state.cursor) {
        format!("Jump to {}", region)
    } else {
        "No region selected".to_string()
    };

    let footer_line = Paragraph::new(Line::from(Span::styled(
        footer,
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(footer_line, chunks[2]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
