use crate::app::{App, MAX_REGION_SHORTCUTS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    let Some(state) = app.region_shortcut_state() else {
        return;
    };

    let area = centered_rect(60, 70, f.area());
    f.render_widget(Clear, area);

    let title = format!(" Region Shortcuts (max {}) ", MAX_REGION_SHORTCUTS);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
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
        Span::styled("SPACE", Style::default().fg(Color::Yellow)),
        Span::raw(" toggle  "),
        Span::styled("ENTER", Style::default().fg(Color::Yellow)),
        Span::raw(" save  "),
        Span::styled("ESC", Style::default().fg(Color::Yellow)),
        Span::raw(" cancel"),
    ]));
    f.render_widget(instructions, chunks[0]);

    let rows = app.available_regions.iter().map(|region| {
        let is_selected = state.selection.iter().any(|selected| selected == region);
        let marker = if is_selected { "[x]" } else { "[ ]" };
        let style = if region == &app.region {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::White)
        };
        Row::new(vec![Cell::from(format!("{} {}", marker, region)).style(style)])
    });

    let table = Table::new(rows, [Constraint::Percentage(100)]).row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    let mut table_state = TableState::default();
    if app.available_regions.is_empty() {
        table_state.select(None);
    } else {
        let max_index = app.available_regions.len().saturating_sub(1);
        table_state.select(Some(state.cursor.min(max_index)));
    }
    f.render_stateful_widget(table, chunks[1], &mut table_state);

    let message = state
        .message
        .clone()
        .unwrap_or_else(|| format!("Select up to {} regions", MAX_REGION_SHORTCUTS));

    let message_line = Paragraph::new(Line::from(Span::styled(
        message,
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(message_line, chunks[2]);
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
