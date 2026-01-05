mod app;
mod aws;
mod config;
mod event;
mod resource;
mod ui;

/// Version injected at compile time via TAWS_VERSION env var (set by CI/CD),
/// or "dev" for local builds.
pub const VERSION: &str = match option_env!("TAWS_VERSION") {
    Some(v) => v,
    None => "dev",
};

use anyhow::Result;
use app::{App, Mode, SsoLoginState};
use aws::client::ClientResult;
use clap::{Parser, ValueEnum};
use config::Config;
use crossterm::{
    event::{poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use ui::splash::{SplashState, render as render_splash};

/// Terminal UI for AWS
#[derive(Parser, Debug)]
#[command(name = "taws", version, about, long_about = None)]
struct Args {
    /// AWS profile to use
    #[arg(short, long)]
    profile: Option<String>,

    /// AWS region to use
    #[arg(short, long)]
    region: Option<String>,

    /// Log level for debugging (logs to platform config dir: Linux ~/.config/taws/taws.log, macOS ~/Library/Application Support/taws/taws.log, Windows %APPDATA%/taws/taws.log)
    #[arg(long, value_enum, default_value = "off")]
    log_level: LogLevel,

    /// Run in read-only mode (block all write operations)
    #[arg(long)]
    readonly: bool,

    /// Custom AWS endpoint URL (for LocalStack, etc.). Also reads from AWS_ENDPOINT_URL env var.
    #[arg(long)]
    endpoint_url: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn to_tracing_level(self) -> Option<Level> {
        match self {
            LogLevel::Off => None,
            LogLevel::Error => Some(Level::ERROR),
            LogLevel::Warn => Some(Level::WARN),
            LogLevel::Info => Some(Level::INFO),
            LogLevel::Debug => Some(Level::DEBUG),
            LogLevel::Trace => Some(Level::TRACE),
        }
    }
}

fn setup_logging(level: LogLevel) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let Some(tracing_level) = level.to_tracing_level() else {
        return None;
    };

    // Get log file path
    let log_path = get_log_path();
    
    // Ensure parent directory exists
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Create file appender
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("Failed to open log file");

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    tracing_subscriber::fmt()
        .with_max_level(tracing_level)
        .with_writer(non_blocking.with_max_level(tracing_level))
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("taws started with log level: {:?}", level);
    tracing::info!("Log file: {:?}", log_path);

    Some(guard)
}

fn get_log_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        return config_dir.join("taws").join("taws.log");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".taws").join("taws.log");
    }
    PathBuf::from("taws.log")
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Setup logging (keep guard alive for the duration of the program)
    let _log_guard = setup_logging(args.log_level);

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Show splash screen and initialize
    let result = initialize_with_splash(&mut terminal, &args).await;

    match result {
        Ok(Some(mut app)) => {
            // Run the main app
            let run_result = run_app(&mut terminal, &mut app).await;

            // Restore terminal
            cleanup_terminal(&mut terminal)?;

            if let Err(err) = run_result {
                eprintln!("Error: {err:?}");
            }
        }
        Ok(None) => {
            // User aborted during initialization
            cleanup_terminal(&mut terminal)?;
        }
        Err(err) => {
            // Restore terminal before showing error
            cleanup_terminal(&mut terminal)?;
            eprintln!("Initialization error: {err:?}");
        }
    }

    Ok(())
}

fn cleanup_terminal<B: Backend + std::io::Write>(terminal: &mut Terminal<B>) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Result of initialization - either an App or SSO login is required
enum InitResult {
    App(App),
    SsoRequired {
        profile: String,
        sso_session: String,
        region: String,
        endpoint_url: Option<String>,
        config: Config,
        available_profiles: Vec<String>,
        available_regions: Vec<String>,
        readonly: bool,
    },
}

async fn initialize_with_splash<B: Backend>(terminal: &mut Terminal<B>, args: &Args) -> Result<Option<App>>
where
    B::Error: Send + Sync + 'static,
{
    match initialize_inner(terminal, args).await? {
        None => Ok(None), // User aborted
        Some(InitResult::App(app)) => Ok(Some(app)),
        Some(InitResult::SsoRequired { 
            profile, 
            sso_session, 
            region, 
            endpoint_url, 
            config, 
            available_profiles, 
            available_regions, 
            readonly,
        }) => {
            // Handle SSO login flow
            handle_sso_login_flow(
                terminal, 
                profile, 
                sso_session, 
                region, 
                endpoint_url, 
                config, 
                available_profiles, 
                available_regions,
                readonly,
            ).await
        }
    }
}

async fn initialize_inner<B: Backend>(terminal: &mut Terminal<B>, args: &Args) -> Result<Option<InitResult>>
where
    B::Error: Send + Sync + 'static,
{
    let mut splash = SplashState::new();

    // Render initial splash
    terminal.draw(|f| render_splash(f, &splash))?;

    // Check for abort
    if check_abort()? {
        return Ok(None);
    }

    // Step 1: Load configuration (CLI args > env vars > saved config)
    let config = Config::load();
    let profile = args.profile.clone()
        .unwrap_or_else(|| config.effective_profile());
    let region = args.region.clone()
        .unwrap_or_else(|| config.effective_region());
    
    // Get endpoint URL from CLI arg or environment variable
    let endpoint_url = args.endpoint_url.clone()
        .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok());
    
    tracing::info!("Using profile: {}, region: {}, endpoint_url: {:?}", profile, region, endpoint_url);
    
    splash.set_message(&format!("Loading AWS config [profile: {}]", profile));
    terminal.draw(|f| render_splash(f, &splash))?;
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 2: Load profiles early (needed for SSO flow too)
    splash.set_message("Reading ~/.aws/config");
    terminal.draw(|f| render_splash(f, &splash))?;

    let available_profiles = aws::profiles::list_profiles().unwrap_or_else(|_| vec!["default".to_string()]);
    let available_regions = aws::profiles::list_regions();
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 3: Initialize all AWS clients (check for SSO requirement)
    splash.set_message(&format!("Connecting to AWS services [{}]", region));
    terminal.draw(|f| render_splash(f, &splash))?;

    let client_result = aws::client::AwsClients::new_with_sso_check(&profile, &region, endpoint_url.clone()).await?;
    
    let (clients, actual_region) = match client_result {
        ClientResult::Ok(clients, actual_region) => (clients, actual_region),
        ClientResult::SsoLoginRequired { profile, sso_session, region, endpoint_url } => {
            // SSO login required - return early to handle in separate flow
            return Ok(Some(InitResult::SsoRequired {
                profile,
                sso_session,
                region,
                endpoint_url,
                config,
                available_profiles,
                available_regions,
                readonly: args.readonly,
            }));
        }
    };
    
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 4: Fetch EC2 instances using new dynamic system
    splash.set_message(&format!("Fetching instances from {}", actual_region));
    terminal.draw(|f| render_splash(f, &splash))?;

    let (instances, initial_error) = {
        // Use the new JSON-driven resource system
        match resource::fetch_resources("ec2-instances", &clients, &[]).await {
            Ok(items) => (items, None),
            Err(e) => {
                let error_msg = aws::client::format_aws_error(&e);
                (Vec::new(), Some(error_msg))
            }
        }
    };

    splash.complete_step();
    splash.set_message("Ready!");
    terminal.draw(|f| render_splash(f, &splash))?;

    // Small delay to show completion
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create the app with config
    let mut app = App::from_initialized(
        clients,
        profile,
        actual_region,
        available_profiles,
        available_regions,
        instances,
        config,
        args.readonly,
        endpoint_url,
    );

    // Set initial error if any
    if let Some(err) = initial_error {
        app.error_message = Some(err);
    }

    Ok(Some(InitResult::App(app)))
}

/// Handle SSO login flow interactively
async fn handle_sso_login_flow<B: Backend>(
    terminal: &mut Terminal<B>,
    profile: String,
    sso_session: String,
    region: String,
    endpoint_url: Option<String>,
    config: Config,
    available_profiles: Vec<String>,
    available_regions: Vec<String>,
    readonly: bool,
) -> Result<Option<App>>
where
    B::Error: Send + Sync + 'static,
{
    use aws::sso;
    
    // Create a minimal app state for the SSO dialog
    let mut sso_state = SsoLoginState::Prompt {
        profile: profile.clone(),
        sso_session: sso_session.clone(),
    };
    
    loop {
        // Render SSO dialog
        terminal.draw(|f| {
            render_sso_standalone(f, &sso_state);
        })?;
        
        // Handle input
        if poll(Duration::from_millis(100))? {
            if let Event::Key(key) = read()? {
                match &sso_state {
                    SsoLoginState::Prompt { profile, .. } => {
                        match key.code {
                            KeyCode::Enter => {
                                // First check if we already have a valid cached token (e.g., from aws sso login)
                                let profile_clone = profile.clone();
                                
                                enum SsoStartResult {
                                    ExistingToken(String),
                                    NeedAuth { profile: String, device_auth: sso::DeviceAuthInfo, sso_region: String },
                                    Error(String),
                                }
                                
                                let result = tokio::task::spawn_blocking(move || {
                                    let sso_config = match sso::get_sso_config(&profile_clone) {
                                        Some(c) => c,
                                        None => return SsoStartResult::Error(format!("SSO config not found for profile '{}'", profile_clone)),
                                    };
                                    
                                    // Check for existing valid token first
                                    if let Some(_token) = sso::check_existing_token(&sso_config) {
                                        return SsoStartResult::ExistingToken(profile_clone);
                                    }
                                    
                                    // No valid token, start device authorization
                                    match sso::start_device_authorization(&sso_config) {
                                        Ok(device_auth) => {
                                            // Open browser
                                            let _ = sso::open_sso_browser(&device_auth.verification_uri_complete);
                                            SsoStartResult::NeedAuth { 
                                                profile: profile_clone, 
                                                device_auth, 
                                                sso_region: sso_config.sso_region 
                                            }
                                        }
                                        Err(e) => SsoStartResult::Error(format!("Failed to start SSO: {}", e)),
                                    }
                                }).await?;
                                
                                match result {
                                    SsoStartResult::ExistingToken(prof) => {
                                        // Already have valid token, skip straight to success
                                        sso_state = SsoLoginState::Success { profile: prof };
                                    }
                                    SsoStartResult::NeedAuth { profile: prof, device_auth, sso_region } => {
                                        sso_state = SsoLoginState::WaitingForAuth {
                                            profile: prof,
                                            user_code: device_auth.user_code,
                                            verification_uri: device_auth.verification_uri,
                                            device_code: device_auth.device_code,
                                            interval: device_auth.interval as u64,
                                            sso_region,
                                        };
                                    }
                                    SsoStartResult::Error(e) => {
                                        sso_state = SsoLoginState::Failed { error: e };
                                    }
                                }
                            }
                            KeyCode::Esc | KeyCode::Char('q') => {
                                return Ok(None); // User cancelled
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    SsoLoginState::WaitingForAuth { profile, .. } => {
                        match key.code {
                            KeyCode::Esc => {
                                return Ok(None); // User cancelled
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {
                                // Any other key - continue polling
                            }
                        }
                        
                        // Poll for token - run blocking code on separate thread
                        let profile_clone = profile.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            if let Some(sso_config) = sso::get_sso_config(&profile_clone) {
                                match sso::poll_for_token(&sso_config) {
                                    Ok(Some(_token)) => Ok(Some(profile_clone)),
                                    Ok(None) => Ok(None),
                                    Err(e) => Err(e.to_string()),
                                }
                            } else {
                                Ok(None)
                            }
                        }).await?;
                        
                        match result {
                            Ok(Some(prof)) => {
                                sso_state = SsoLoginState::Success { profile: prof };
                            }
                            Ok(None) => {
                                // Still pending
                            }
                            Err(e) => {
                                sso_state = SsoLoginState::Failed { error: e };
                            }
                        }
                    }
                    SsoLoginState::Success { profile: _sso_profile } => {
                        // Note: _sso_profile should match the outer `profile` variable for initial SSO
                        match key.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                // SSO successful - now create the client and continue initialization
                                // AwsClients::new handles blocking internally via spawn_blocking
                                let (clients, actual_region) = aws::client::AwsClients::new(&profile, &region, endpoint_url.clone()).await?;
                                
                                // Fetch initial resources
                                let (instances, initial_error) = {
                                    match resource::fetch_resources("ec2-instances", &clients, &[]).await {
                                        Ok(items) => (items, None),
                                        Err(e) => {
                                            let error_msg = aws::client::format_aws_error(&e);
                                            (Vec::new(), Some(error_msg))
                                        }
                                    }
                                };
                                
                                let mut app = App::from_initialized(
                                    clients,
                                    profile,
                                    actual_region,
                                    available_profiles,
                                    available_regions,
                                    instances,
                                    config,
                                    readonly,
                                    endpoint_url,
                                );
                                
                                if let Some(err) = initial_error {
                                    app.error_message = Some(err);
                                }
                                
                                return Ok(Some(app));
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    SsoLoginState::Failed { .. } => {
                        match key.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                return Ok(None); // Exit on failure
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                }
            }
        } else {
            // No key event - poll for SSO if waiting
            if let SsoLoginState::WaitingForAuth { profile: waiting_profile, .. } = &sso_state {
                let waiting_profile = waiting_profile.clone();
                let result = tokio::task::spawn_blocking(move || {
                    if let Some(sso_config) = sso::get_sso_config(&waiting_profile) {
                        match sso::poll_for_token(&sso_config) {
                            Ok(Some(_token)) => Ok(Some(waiting_profile)),
                            Ok(None) => Ok(None),
                            Err(e) => Err(e.to_string()),
                        }
                    } else {
                        Ok(None)
                    }
                }).await?;
                
                match result {
                    Ok(Some(prof)) => {
                        sso_state = SsoLoginState::Success { profile: prof };
                    }
                    Ok(None) => {
                        // Still pending
                    }
                    Err(e) => {
                        sso_state = SsoLoginState::Failed { error: e };
                    }
                }
            }
        }
    }
}

/// Render SSO dialog standalone (during initialization, before app is created)
fn render_sso_standalone(f: &mut ratatui::Frame, sso_state: &SsoLoginState) {
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout, Rect},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };
    
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
    
    // Clear the screen with a dark background
    let area = f.area();
    f.render_widget(Clear, area);
    let bg_block = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(bg_block, area);
    
    match sso_state {
        SsoLoginState::Prompt { profile, sso_session } => {
            let dialog_area = centered_rect(70, 10, area);
            f.render_widget(Clear, dialog_area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Required>",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
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

            let paragraph = Paragraph::new(text).block(block).alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }

        SsoLoginState::WaitingForAuth { user_code, verification_uri, .. } => {
            let dialog_area = centered_rect(70, 12, area);
            f.render_widget(Clear, dialog_area);

            let text = vec![
                Line::from(Span::styled(
                    "<Waiting for SSO Authentication>",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Complete authentication in your browser.",
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Code: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(user_code, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
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

            let paragraph = Paragraph::new(text).block(block).alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }

        SsoLoginState::Success { profile } => {
            let dialog_area = centered_rect(50, 7, area);
            f.render_widget(Clear, dialog_area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Successful>",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Authenticated '{}'. Press Enter to continue.", profile),
                    Style::default().fg(Color::White),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));

            let paragraph = Paragraph::new(text).block(block).alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }

        SsoLoginState::Failed { error } => {
            let dialog_area = centered_rect(70, 9, area);
            f.render_widget(Clear, dialog_area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Failed>",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(error.as_str(), Style::default().fg(Color::White))),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter or Esc to exit",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let paragraph = Paragraph::new(text).block(block).alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }
    }
}

fn check_abort() -> Result<bool> {
    if poll(Duration::from_millis(50))? {
        if let Event::Key(key) = read()? {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        // Handle user input
        if event::handle_events(app).await? {
            return Ok(());
        }
        
        // Poll SSO if in waiting state
        if app.mode == Mode::SsoLogin {
            event::poll_sso_if_waiting(app).await;
        }
        
        // Poll for new log events if in log tail mode
        if app.mode == Mode::LogTail {
            event::poll_logs_if_tailing(app).await;
        }
        
        // Auto-refresh every 5 seconds (only in Normal mode)
        if app.needs_refresh() {
            let _ = app.refresh_current().await;
        }
    }
}
