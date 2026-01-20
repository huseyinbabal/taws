use crate::app::{App, Mode, SsoLoginState};
use crate::aws::sso;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::Duration;

pub async fn handle_events(app: &mut App) -> Result<bool> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // Only handle key press events, not release or repeat
            // This fixes double key presses on Windows
            if key.kind != KeyEventKind::Press {
                return Ok(false);
            }
            return handle_key_event(app, key).await;
        }
    }
    Ok(false)
}

async fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<bool> {
    match app.mode {
        Mode::Normal => handle_normal_mode(app, key).await,
        Mode::Command => handle_command_mode(app, key).await,
        Mode::Help => handle_help_mode(app, key),
        Mode::Describe => handle_describe_mode(app, key),
        Mode::Confirm => handle_confirm_mode(app, key).await,
        Mode::Warning => handle_warning_mode(app, key),
        Mode::Profiles => handle_profiles_mode(app, key).await,
        Mode::Regions => handle_regions_mode(app, key).await,
        Mode::SsoLogin => handle_sso_login_mode(app, key).await,
        Mode::LogTail => handle_log_tail_mode(app, key).await,
    }
}

// Default region shortcuts (used when no recent history)
const DEFAULT_REGIONS: &[&str] = &[
    "us-east-1",
    "us-west-2",
    "eu-west-1",
    "eu-central-1",
    "ap-northeast-1",
    "ap-southeast-1",
];

/// Get region for shortcut index (from recent + defaults to fill 6 slots)
fn get_region_for_shortcut(app: &App, index: usize) -> Option<String> {
    get_region_shortcuts(app).get(index).cloned()
}

/// Build list of 6 region shortcuts: recent regions first, then defaults to fill remaining slots
fn get_region_shortcuts(app: &App) -> Vec<String> {
    let recent = app.config.get_recent_regions();
    let mut regions: Vec<String> = recent.clone();

    // Fill remaining slots with defaults (excluding any already in the list)
    for default in DEFAULT_REGIONS {
        if regions.len() >= 6 {
            break;
        }
        if !regions.iter().any(|r| r == *default) {
            regions.push(default.to_string());
        }
    }

    regions
}

async fn handle_normal_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    // If filter is active, handle filter input
    if app.filter_active {
        return handle_filter_input(app, key).await;
    }

    match key.code {
        // Quit with Ctrl+C
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),

        // Region shortcuts (0-5)
        KeyCode::Char('0') => {
            if let Some(region) = get_region_for_shortcut(app, 0) {
                app.switch_region(&region).await?;
                app.refresh_current().await?;
            }
        }
        KeyCode::Char('1') => {
            if let Some(region) = get_region_for_shortcut(app, 1) {
                app.switch_region(&region).await?;
                app.refresh_current().await?;
            }
        }
        KeyCode::Char('2') => {
            if let Some(region) = get_region_for_shortcut(app, 2) {
                app.switch_region(&region).await?;
                app.refresh_current().await?;
            }
        }
        KeyCode::Char('3') => {
            if let Some(region) = get_region_for_shortcut(app, 3) {
                app.switch_region(&region).await?;
                app.refresh_current().await?;
            }
        }
        KeyCode::Char('4') => {
            if let Some(region) = get_region_for_shortcut(app, 4) {
                app.switch_region(&region).await?;
                app.refresh_current().await?;
            }
        }
        KeyCode::Char('5') => {
            if let Some(region) = get_region_for_shortcut(app, 5) {
                app.switch_region(&region).await?;
                app.refresh_current().await?;
            }
        }

        // Navigation - vim style
        KeyCode::Char('j') | KeyCode::Down => app.next(),
        KeyCode::Char('k') | KeyCode::Up => app.previous(),
        KeyCode::Home => app.go_to_top(),
        KeyCode::Char('G') | KeyCode::End => app.go_to_bottom(),

        // Page navigation
        KeyCode::PageUp | KeyCode::Char('b')
            if key.code == KeyCode::PageUp || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.page_up(10);
        }
        KeyCode::PageDown | KeyCode::Char('f')
            if key.code == KeyCode::PageDown || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.page_down(10);
        }

        // Destructive action (ctrl+d)
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(resource) = app.current_resource() {
                for action in &resource.actions {
                    if action.shortcut.as_deref() == Some("ctrl+d") {
                        if let Some(item) = app.selected_item() {
                            let id = crate::resource::extract_json_value(item, &resource.id_field);
                            if id != "-" && !id.is_empty() {
                                if app.readonly {
                                    app.show_warning(
                                        "This operation is not supported in read-only mode",
                                    );
                                } else if let Some(pending) = app.create_pending_action(action, &id)
                                {
                                    app.enter_confirm_mode(pending);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // Describe mode (d or Enter)
        KeyCode::Char('d') => app.enter_describe_mode().await,
        KeyCode::Enter => app.enter_describe_mode().await,

        // Filter toggle
        KeyCode::Char('/') => {
            app.toggle_filter();
        }

        // Pagination - next/previous page of results (using ] and [ to avoid conflicts with sub-resource shortcuts)
        KeyCode::Char(']') => {
            if app.pagination.has_more {
                app.next_page().await?;
            }
        }
        KeyCode::Char('[') => {
            if app.pagination.current_page > 1 {
                app.prev_page().await?;
            }
        }

        // Manual refresh
        KeyCode::Char('R') => {
            app.reset_pagination();
            app.refresh_current().await?;
        }

        // Mode switches
        KeyCode::Char(':') => app.enter_command_mode(),
        KeyCode::Char('?') => app.enter_help_mode(),

        // Backspace goes back in navigation
        KeyCode::Backspace => {
            if app.parent_context.is_some() {
                app.navigate_back().await?;
            }
        }

        // Escape clears filter if present
        KeyCode::Esc => {
            if !app.filter_text.is_empty() {
                app.clear_filter();
            } else if app.parent_context.is_some() {
                app.navigate_back().await?;
            }
        }

        // Dynamic shortcuts: sub-resources and EC2 actions
        _ => {
            if let KeyCode::Char(c) = key.code {
                let mut handled = false;

                // Check if it's a sub-resource shortcut for current resource
                if let Some(resource) = app.current_resource() {
                    for sub in &resource.sub_resources {
                        if sub.shortcut == c.to_string() && app.selected_item().is_some() {
                            app.navigate_to_sub_resource(&sub.resource_key).await?;
                            handled = true;
                            break;
                        }
                    }
                }

                // Check if it matches an action shortcut
                if !handled {
                    if let Some(resource) = app.current_resource() {
                        for action in &resource.actions {
                            if action.shortcut.as_deref() == Some(&c.to_string()) {
                                if let Some(item) = app.selected_item() {
                                    let id = crate::resource::extract_json_value(
                                        item,
                                        &resource.id_field,
                                    );
                                    if id != "-" && !id.is_empty() {
                                        // Special handling for log tailing action
                                        if action.sdk_method == "tail_logs" {
                                            app.enter_log_tail_mode().await?;
                                            handled = true;
                                        // Special handling for SSM connect
                                        } else if action.sdk_method == "ssm_connect" {
                                            app.request_ssm_connect();
                                            handled = true;
                                        } else if action.show_result {
                                            // Action that displays result (e.g., get_secret_value)
                                            // These are read-only operations (retrieve and display data),
                                            // so they're allowed even in readonly mode
                                            match crate::resource::execute_action_with_result(
                                                &resource.service,
                                                &action.sdk_method,
                                                &app.clients,
                                                &id,
                                            )
                                            .await
                                            {
                                                Ok(data) => {
                                                    app.describe_data = Some(data);
                                                    app.describe_scroll = 0;
                                                    app.last_action_display_name =
                                                        Some(action.display_name.clone());
                                                    app.mode = crate::app::Mode::Describe;
                                                }
                                                Err(e) => {
                                                    app.error_message =
                                                        Some(format!("Action failed: {}", e));
                                                }
                                            }
                                            handled = true;
                                        // Block mutating actions in readonly mode
                                        } else if app.readonly {
                                            app.show_warning(
                                                "This operation is not supported in read-only mode",
                                            );
                                            handled = true;
                                        } else if action.requires_confirm() {
                                            // Check if action requires confirmation
                                            if let Some(pending) =
                                                app.create_pending_action(action, &id)
                                            {
                                                app.enter_confirm_mode(pending);
                                                handled = true;
                                            }
                                        } else {
                                            // Execute directly
                                            if let Err(e) = crate::resource::execute_action(
                                                &resource.service,
                                                &action.sdk_method,
                                                &app.clients,
                                                &id,
                                            )
                                            .await
                                            {
                                                app.error_message =
                                                    Some(format!("Action failed: {}", e));
                                            }
                                            let _ = app.refresh_current().await;
                                            handled = true;
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }

                // Handle 'gg' for go_to_top
                if c == 'g' {
                    if let Some((last_key, last_time)) = app.last_key_press {
                        if last_key == KeyCode::Char('g')
                            && last_time.elapsed() < Duration::from_millis(250)
                        {
                            app.go_to_top();
                            app.last_key_press = None;
                            handled = true;
                        }
                    }
                }
                if !handled && c == 'g' {
                    app.last_key_press = Some((KeyCode::Char('g'), std::time::Instant::now()));
                } else {
                    app.last_key_press = None;
                }
            }
        }
    }
    Ok(false)
}

async fn handle_filter_input(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.clear_filter();
        }
        KeyCode::Enter => {
            app.filter_active = false;
        }
        KeyCode::Backspace => {
            app.filter_text.pop();
            app.apply_filter();
        }
        KeyCode::Char(c) => {
            app.filter_text.push(c);
            app.apply_filter();
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_command_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.command_text.clear();
            app.exit_mode();
        }
        KeyCode::Enter => {
            let should_quit = app.execute_command().await?;
            if should_quit {
                return Ok(true);
            }
            if app.mode == Mode::Command {
                app.exit_mode();
            }
        }
        KeyCode::Tab | KeyCode::Right => {
            app.apply_suggestion();
        }
        KeyCode::Down => {
            app.next_suggestion();
        }
        KeyCode::Up => {
            app.prev_suggestion();
        }
        KeyCode::Backspace => {
            app.command_text.pop();
            app.update_command_suggestions();
        }
        KeyCode::Char(c) => {
            app.command_text.push(c);
            app.update_command_suggestions();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_help_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            app.exit_mode();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_describe_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    // If search input is active, handle text input
    if app.describe_search_active {
        return handle_describe_search_input(app, key);
    }

    // Page size for PageUp/PageDown and Ctrl+b/Ctrl+f
    const PAGE_SIZE: usize = 20;

    match key.code {
        KeyCode::Esc => {
            if !app.describe_search_text.is_empty() {
                // Clear search first
                app.clear_describe_search();
            } else {
                app.exit_mode();
            }
        }
        KeyCode::Char('q') | KeyCode::Char('d') => {
            app.clear_describe_search();
            app.exit_mode();
        }
        // Start search with '/'
        KeyCode::Char('/') => {
            app.describe_search_active = true;
        }
        // Next match with 'n'
        KeyCode::Char('n') => {
            app.describe_next_match();
        }
        // Previous match with 'N'
        KeyCode::Char('N') => {
            app.describe_prev_match();
        }
        // Page down with Ctrl+f or PageDown
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.describe_scroll_down(PAGE_SIZE);
        }
        KeyCode::PageDown => {
            app.describe_scroll_down(PAGE_SIZE);
        }
        // Page up with Ctrl+b or PageUp
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.describe_scroll_up(PAGE_SIZE);
        }
        KeyCode::PageUp => {
            app.describe_scroll_up(PAGE_SIZE);
        }
        // Single line navigation
        KeyCode::Char('j') | KeyCode::Down => {
            app.describe_scroll_down(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.describe_scroll_up(1);
        }
        // Go to top
        KeyCode::Char('g') | KeyCode::Home => {
            app.describe_scroll = 0;
        }
        // Go to bottom
        KeyCode::Char('G') | KeyCode::End => {
            app.describe_scroll_to_bottom(40);
        }
        _ => {}
    }
    Ok(false)
}

fn handle_describe_search_input(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            // Cancel search input, keep existing search text if any
            app.describe_search_active = false;
        }
        KeyCode::Enter => {
            // Confirm search, exit input mode
            app.describe_search_active = false;
        }
        KeyCode::Backspace => {
            app.describe_search_text.pop();
            app.update_describe_search();
        }
        KeyCode::Char(c) => {
            app.describe_search_text.push(c);
            app.update_describe_search();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_warning_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('O') => {
            app.warning_message = None;
            app.exit_mode();
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_confirm_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        // Toggle selection with arrow keys or tab
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char('h') | KeyCode::Char('l') => {
            if let Some(ref mut pending) = app.pending_action {
                pending.selected_yes = !pending.selected_yes;
            }
        }
        // Confirm with Enter
        KeyCode::Enter => {
            if let Some(ref pending) = app.pending_action {
                if pending.selected_yes {
                    // Execute the action (if not in readonly mode)
                    if app.readonly {
                        app.error_message =
                            Some("This operation is not supported in read-only mode".to_string());
                    } else {
                        let service = pending.service.clone();
                        let method = pending.sdk_method.clone();
                        let resource_id = pending.resource_id.clone();

                        if let Err(e) = crate::resource::execute_action(
                            &service,
                            &method,
                            &app.clients,
                            &resource_id,
                        )
                        .await
                        {
                            app.error_message = Some(format!("Action failed: {}", e));
                        }
                        // Refresh after action
                        let _ = app.refresh_current().await;
                    }
                }
            }
            app.exit_mode();
        }
        // Quick yes/no
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if app.readonly {
                app.error_message =
                    Some("This operation is not supported in read-only mode".to_string());
            } else if let Some(ref pending) = app.pending_action {
                let service = pending.service.clone();
                let method = pending.sdk_method.clone();
                let resource_id = pending.resource_id.clone();

                if let Err(e) =
                    crate::resource::execute_action(&service, &method, &app.clients, &resource_id)
                        .await
                {
                    app.error_message = Some(format!("Action failed: {}", e));
                }
                let _ = app.refresh_current().await;
            }
            app.exit_mode();
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.exit_mode();
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_profiles_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_mode();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.previous();
        }
        KeyCode::Char('g') | KeyCode::Home => {
            app.go_to_top();
        }
        KeyCode::Char('G') | KeyCode::End => {
            app.go_to_bottom();
        }
        KeyCode::Enter => {
            app.select_profile().await?;
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_regions_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_mode();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.previous();
        }
        KeyCode::Char('g') | KeyCode::Home => {
            app.go_to_top();
        }
        KeyCode::Char('G') | KeyCode::End => {
            app.go_to_bottom();
        }
        KeyCode::Enter => {
            app.select_region().await?;
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_sso_login_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    let sso_state = match &app.sso_state {
        Some(state) => state.clone(),
        None => {
            app.exit_mode();
            return Ok(false);
        }
    };

    match sso_state {
        SsoLoginState::Prompt {
            profile,
            sso_session: _,
        } => {
            match key.code {
                KeyCode::Enter => {
                    // Get SSO config and start device authorization - run blocking on separate thread
                    let profile_clone = profile.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        if let Some(config) = sso::get_sso_config(&profile_clone) {
                            match sso::start_device_authorization(&config) {
                                Ok(device_auth) => {
                                    // Open browser
                                    let _ = sso::open_sso_browser(
                                        &device_auth.verification_uri_complete,
                                    );
                                    Ok((profile_clone, device_auth, config.sso_region))
                                }
                                Err(e) => Err(format!("Failed to start SSO: {}", e)),
                            }
                        } else {
                            Err(format!(
                                "SSO config not found for profile '{}'",
                                profile_clone
                            ))
                        }
                    })
                    .await;

                    match result {
                        Ok(Ok((prof, device_auth, sso_region))) => {
                            app.sso_state = Some(SsoLoginState::WaitingForAuth {
                                profile: prof,
                                user_code: device_auth.user_code,
                                verification_uri: device_auth.verification_uri,
                                device_code: device_auth.device_code,
                                interval: device_auth.interval as u64,
                                sso_region,
                            });
                        }
                        Ok(Err(e)) => {
                            app.sso_state = Some(SsoLoginState::Failed { error: e });
                        }
                        Err(e) => {
                            app.sso_state = Some(SsoLoginState::Failed {
                                error: format!("Task failed: {}", e),
                            });
                        }
                    }
                }
                KeyCode::Esc => {
                    app.sso_state = None;
                    app.exit_mode();
                }
                _ => {}
            }
        }

        SsoLoginState::WaitingForAuth {
            profile,
            interval: _,
            ..
        } => {
            match key.code {
                KeyCode::Esc => {
                    app.sso_state = None;
                    app.exit_mode();
                }
                _ => {
                    // Poll for token - run blocking on separate thread
                    let profile_clone = profile.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        if let Some(config) = sso::get_sso_config(&profile_clone) {
                            match sso::poll_for_token(&config) {
                                Ok(Some(_token)) => Ok(Some(profile_clone)),
                                Ok(None) => Ok(None),
                                Err(e) => Err(e.to_string()),
                            }
                        } else {
                            Ok(None)
                        }
                    })
                    .await;

                    match result {
                        Ok(Ok(Some(prof))) => {
                            app.sso_state = Some(SsoLoginState::Success { profile: prof });
                        }
                        Ok(Ok(None)) => {
                            // Still pending
                        }
                        Ok(Err(e)) => {
                            app.sso_state = Some(SsoLoginState::Failed { error: e });
                        }
                        Err(e) => {
                            app.sso_state = Some(SsoLoginState::Failed {
                                error: format!("Task failed: {}", e),
                            });
                        }
                    }
                }
            }
        }

        SsoLoginState::Success { profile } => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    // Now complete the profile switch with fresh SSO credentials
                    let profile_to_switch = profile.clone();
                    app.sso_state = None;
                    app.exit_mode();
                    // Actually switch the profile now that SSO is complete
                    if let Err(e) = app.switch_profile(&profile_to_switch).await {
                        app.error_message = Some(format!("Failed to switch profile: {}", e));
                    } else {
                        let _ = app.refresh_current().await;
                    }
                }
                _ => {}
            }
        }

        SsoLoginState::Failed { .. } => match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                app.sso_state = None;
                app.exit_mode();
            }
            _ => {}
        },
    }

    Ok(false)
}

/// Poll SSO token in background (called from main loop when in SSO waiting state)
pub async fn poll_sso_if_waiting(app: &mut App) {
    if app.mode != Mode::SsoLogin {
        return;
    }

    let sso_state = match &app.sso_state {
        Some(state) => state.clone(),
        None => return,
    };

    if let SsoLoginState::WaitingForAuth { profile, .. } = sso_state {
        let profile_clone = profile.clone();
        let result = tokio::task::spawn_blocking(move || {
            if let Some(config) = sso::get_sso_config(&profile_clone) {
                match sso::poll_for_token(&config) {
                    Ok(Some(_token)) => Ok(Some(profile_clone)),
                    Ok(None) => Ok(None),
                    Err(e) => Err(e.to_string()),
                }
            } else {
                Ok(None)
            }
        })
        .await;

        match result {
            Ok(Ok(Some(prof))) => {
                app.sso_state = Some(SsoLoginState::Success { profile: prof });
            }
            Ok(Ok(None)) => {
                // Still pending
            }
            Ok(Err(e)) => {
                app.sso_state = Some(SsoLoginState::Failed { error: e });
            }
            Err(e) => {
                app.sso_state = Some(SsoLoginState::Failed {
                    error: format!("Task failed: {}", e),
                });
            }
        }
    }
}

async fn handle_log_tail_mode(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        // Exit log tail mode
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_log_tail_mode();
        }
        // Scroll up
        KeyCode::Char('k') | KeyCode::Up => {
            app.log_tail_scroll_up(1);
        }
        // Scroll down
        KeyCode::Char('j') | KeyCode::Down => {
            app.log_tail_scroll_down(1);
        }
        // Page up
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.log_tail_scroll_up(10);
        }
        // Page down
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.log_tail_scroll_down(10);
        }
        // Go to top
        KeyCode::Char('g') | KeyCode::Home => {
            app.log_tail_scroll_to_top();
        }
        // Go to bottom (and enable auto-scroll)
        KeyCode::Char('G') | KeyCode::End => {
            app.log_tail_scroll_to_bottom();
        }
        // Toggle pause
        KeyCode::Char(' ') => {
            app.toggle_log_tail_pause();
        }
        _ => {}
    }
    Ok(false)
}

/// Poll for new log events if in log tail mode
pub async fn poll_logs_if_tailing(app: &mut App) {
    if app.mode != Mode::LogTail {
        return;
    }

    let should_poll = if let Some(ref state) = app.log_tail_state {
        !state.paused && state.last_poll.elapsed() >= Duration::from_secs(2)
    } else {
        false
    };

    if should_poll {
        let _ = app.poll_log_events().await;
    }
}
