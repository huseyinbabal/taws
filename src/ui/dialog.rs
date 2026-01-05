use crate::app::{App, Mode, SsoLoginState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    match app.mode {
        Mode::Confirm => render_confirm_dialog(f, app),
        Mode::Warning => render_warning_dialog(f, app),
        Mode::SsoLogin => render_sso_dialog(f, app),
        _ => {}
    }
}

fn render_confirm_dialog(f: &mut Frame, app: &App) {
    let Some(pending) = &app.pending_action else {
        return;
    };

    let area = centered_rect(60, 9, f.area());

    f.render_widget(Clear, area);

    // Determine title color based on destructive flag
    let title_color = if pending.destructive {
        Color::Red
    } else {
        Color::Yellow
    };

    let title = if pending.destructive {
        "Delete"
    } else {
        "Confirm"
    };

    // Build Cancel/OK buttons with selection indicator (Cancel = !selected_yes, OK = selected_yes)
    let cancel_style = if !pending.selected_yes {
        Style::default().fg(Color::Black).bg(Color::Magenta)
    } else {
        Style::default().fg(Color::White)
    };

    let ok_style = if pending.selected_yes {
        Style::default().fg(Color::Black).bg(Color::Magenta)
    } else {
        Style::default().fg(Color::White)
    };

    // Build the dialog content
    let text = vec![
        Line::from(Span::styled(
            format!("<{}>", title),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            &pending.message,
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Cancel ", cancel_style),
            Span::raw("    "),
            Span::styled(" OK ", ok_style),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);

    f.render_widget(paragraph, area);
}

fn render_warning_dialog(f: &mut Frame, app: &App) {
    let Some(message) = &app.warning_message else {
        return;
    };

    let area = centered_rect(60, 8, f.area());

    f.render_widget(Clear, area);

    let text = vec![
        Line::from(Span::styled(
            "<Warning>",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![Span::styled(
            " OK ",
            Style::default().fg(Color::Black).bg(Color::Magenta),
        )]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);

    f.render_widget(paragraph, area);
}

fn render_sso_dialog(f: &mut Frame, app: &App) {
    let Some(ref sso_state) = app.sso_state else {
        return;
    };

    match sso_state {
        SsoLoginState::Prompt {
            profile,
            sso_session,
        } => {
            let area = centered_rect(70, 10, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Required>",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Profile '{}' requires SSO authentication.", profile),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    format!("Session: {}", sso_session),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter to open browser for login, Esc to cancel",
                    Style::default().fg(Color::Yellow),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        SsoLoginState::WaitingForAuth {
            user_code,
            verification_uri,
            ..
        } => {
            let area = centered_rect(70, 12, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<Waiting for SSO Authentication>",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Complete authentication in your browser.",
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Code: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        user_code,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("URL: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(verification_uri, Style::default().fg(Color::Blue)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Waiting... (Press Esc to cancel)",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        SsoLoginState::Success { profile } => {
            let area = centered_rect(50, 7, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Successful>",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Authentication complete for '{}'!", profile),
                    Style::default().fg(Color::White),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        SsoLoginState::Failed { error } => {
            let area = centered_rect(70, 9, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Failed>",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    error.as_str(),
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter or Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }
    }
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(height),
            Constraint::Percentage(40),
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
