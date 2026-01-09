use crate::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // Split header into 4 columns like k9s
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(22), // Left: Context info
            Constraint::Percentage(18), // Region/Sub-resource shortcuts
            Constraint::Percentage(22), // Keybindings col 1
            Constraint::Percentage(22), // Keybindings col 2
            Constraint::Percentage(16), // Logo
        ])
        .split(area);

    render_context_column(f, app, columns[0]);
    render_shortcuts_column(f, app, columns[1]);
    render_keybindings_col1(f, app, columns[2]);
    render_keybindings_col2(f, columns[3]);
    render_logo(f, columns[4]);
}

fn render_context_column(f: &mut Frame, app: &App, area: Rect) {
    let resource_name = app
        .current_resource()
        .map(|r| r.display_name.as_str())
        .unwrap_or(&app.current_resource_key);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Profile:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                &app.profile,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Region: ", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                &app.region,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Resource:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                resource_name.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    // Show parent context if navigating
    if let Some(parent) = &app.parent_context {
        lines.push(Line::from(vec![
            Span::styled("Context:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(&parent.display_name, Style::default().fg(Color::Yellow)),
        ]));
    }

    // Show read-only mode indicator
    if app.readonly {
        lines.push(Line::from(vec![
            Span::styled("Mode:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "READONLY",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Show custom endpoint indicator
    if app.endpoint_url.is_some() {
        lines.push(Line::from(vec![
            Span::styled("Endpoint:", Style::default().fg(Color::DarkGray)),
            Span::styled(
                " CUSTOM",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_shortcuts_column(f: &mut Frame, app: &App, area: Rect) {
    // If current resource has sub-resources, show those as shortcuts
    // Otherwise show region shortcuts
    if let Some(resource) = app.current_resource() {
        if !resource.sub_resources.is_empty() {
            render_subresource_shortcuts(f, app, resource, area);
            return;
        }
    }

    render_region_shortcuts(f, app, area);
}

fn render_region_shortcuts(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    if app.region_shortcuts.is_empty() {
        lines.push(Line::from(Span::styled(
            "No shortcuts configured",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (idx, region) in app.region_shortcuts.iter().enumerate() {
            let is_current = region == &app.region;
            let style = if is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("<{}>", idx),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(" "),
                Span::styled(region.clone(), style),
            ]));
        }
    }

    lines.push(Line::from(vec![
        Span::styled(
            "<ctrl+r>",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Edit"),
    ]));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_subresource_shortcuts(
    f: &mut Frame,
    _app: &App,
    resource: &crate::resource::ResourceDef,
    area: Rect,
) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "Sub-resources:",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))];

    for sub in resource.sub_resources.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("<{}>", sub.shortcut),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(" "),
            Span::styled(sub.display_name.clone(), Style::default().fg(Color::White)),
        ]));
    }

    // Show if there are more
    if resource.sub_resources.len() > 5 {
        lines.push(Line::from(Span::styled(
            format!("  +{} more", resource.sub_resources.len() - 5),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_keybindings_col1(f: &mut Frame, app: &App, area: Rect) {
    // Show resource-specific actions or generic bindings
    let bindings: Vec<(String, String)> = if let Some(resource) = app.current_resource() {
        let mut b: Vec<(String, String)> = vec![("<d>".to_string(), "Describe".to_string())];

        // Add resource-specific actions
        for action in resource.actions.iter().take(4) {
            if let Some(ref shortcut) = action.shortcut {
                b.push((format!("<{}>", shortcut), action.display_name.clone()));
            }
        }

        b.push(("<?>".to_string(), "Help".to_string()));
        b
    } else {
        vec![
            ("<d>".to_string(), "Describe".to_string()),
            ("<?>".to_string(), "Help".to_string()),
        ]
    };

    let lines: Vec<Line> = bindings
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(format!("{:<9}", key), Style::default().fg(Color::Yellow)),
                Span::styled(desc.clone(), Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_keybindings_col2(f: &mut Frame, area: Rect) {
    let bindings = [
        ("</>", "Filter"),
        ("<:>", "Resources"),
        ("<esc>", "Back"),
        ("<bs>", "Parent"),
        ("<ctrl-c>", "Quit"),
        ("", ""),
    ];

    let lines: Vec<Line> = bindings
        .iter()
        .map(|(key, desc)| {
            if key.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled(format!("{:<9}", key), Style::default().fg(Color::Yellow)),
                    Span::styled(*desc, Style::default().fg(Color::DarkGray)),
                ])
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_logo(f: &mut Frame, area: Rect) {
    let logo = vec![
        Line::from(Span::styled(
            "▀█▀ ▄▀█ █ █ █ █▀",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " █  █▀█ ▀▄▀▄▀ ▄█",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "AWS TUI",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            crate::VERSION,
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(logo);
    f.render_widget(paragraph, area);
}
