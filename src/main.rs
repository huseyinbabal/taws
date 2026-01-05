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
use app::App;
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

async fn initialize_with_splash<B: Backend>(terminal: &mut Terminal<B>, args: &Args) -> Result<Option<App>>
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
    
    tracing::info!("Using profile: {}, region: {}", profile, region);
    
    splash.set_message(&format!("Loading AWS config [profile: {}]", profile));
    terminal.draw(|f| render_splash(f, &splash))?;
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 2: Initialize all AWS clients
    splash.set_message(&format!("Connecting to AWS services [{}]", region));
    terminal.draw(|f| render_splash(f, &splash))?;

    let (clients, actual_region) = aws::client::AwsClients::new(&profile, &region).await?;
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 3: Load profiles
    splash.set_message("Reading ~/.aws/config");
    terminal.draw(|f| render_splash(f, &splash))?;

    let available_profiles = aws::profiles::list_profiles().unwrap_or_else(|_| vec!["default".to_string()]);
    let available_regions = aws::profiles::list_regions();
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
    );

    // Set initial error if any
    if let Some(err) = initial_error {
        app.error_message = Some(err);
    }

    Ok(Some(app))
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
        
        // Auto-refresh every 5 seconds (only in Normal mode)
        if app.needs_refresh() {
            let _ = app.refresh_current().await;
        }
    }
}
