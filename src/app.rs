use crate::aws;
use crate::aws::client::AwsClients;
use crate::config::Config;
use crossterm::event::KeyCode;
use crate::resource::{
    get_resource, get_all_resource_keys, ResourceDef, ResourceFilter, 
    fetch_resources_paginated, extract_json_value,
};
use anyhow::Result;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,      // Viewing list
    Command,     // : command input
    Help,        // ? help popup
    Confirm,     // Confirmation dialog
    Warning,     // Warning/info dialog (OK only)
    Profiles,    // Profile selection
    Regions,     // Region selection
    Describe,    // Viewing JSON details of selected item
    SsoLogin,    // SSO login dialog
    LogTail,     // Tailing CloudWatch logs
}

/// Pending action that requires confirmation
#[derive(Debug, Clone)]
pub struct PendingAction {
    /// Service name (e.g., "ec2")
    pub service: String,
    /// SDK method to call (e.g., "terminate_instance")  
    pub sdk_method: String,
    /// Resource ID to act on
    pub resource_id: String,
    /// Display message for confirmation dialog
    pub message: String,
    /// If true, default selection is No (kept for potential future use)
    #[allow(dead_code)]
    pub default_no: bool,
    /// If true, show as destructive (red)
    pub destructive: bool,
    /// Currently selected option (true = Yes, false = No)
    pub selected_yes: bool,
}

/// Parent context for hierarchical navigation
#[derive(Debug, Clone)]
pub struct ParentContext {
    /// Parent resource key (e.g., "vpc")
    pub resource_key: String,
    /// Parent item (the selected VPC, etc.)
    pub item: Value,
    /// Display name for breadcrumb
    pub display_name: String,
}

pub struct App {
    // AWS Clients
    pub clients: AwsClients,
    
    // Current resource being viewed
    pub current_resource_key: String,
    
    // Dynamic data storage (JSON)
    pub items: Vec<Value>,
    pub filtered_items: Vec<Value>,
    
    // Navigation state
    pub selected: usize,
    pub mode: Mode,
    pub filter_text: String,
    pub filter_active: bool,
    
    // Hierarchical navigation
    pub parent_context: Option<ParentContext>,
    pub navigation_stack: Vec<ParentContext>,
    
    // Command input
    pub command_text: String,
    pub command_suggestions: Vec<String>,
    pub command_suggestion_selected: usize,
    pub command_preview: Option<String>, // Ghost text for hovered suggestion
    
    // Profile/Region
    pub profile: String,
    pub region: String,
    pub available_profiles: Vec<String>,
    pub available_regions: Vec<String>,
    pub profiles_selected: usize,
    pub regions_selected: usize,
    
    // Confirmation
    pub pending_action: Option<PendingAction>,
    
    // UI state
    pub loading: bool,
    pub error_message: Option<String>,
    pub describe_scroll: usize,
    pub describe_data: Option<Value>,  // Full resource details from describe API
    
    // Auto-refresh
    pub last_refresh: std::time::Instant,
    
    // Persistent configuration
    pub config: Config,
    
    // Key press tracking for sequences (e.g., 'gg')
    pub last_key_press: Option<(KeyCode, std::time::Instant)>,
    
    // Read-only mode (blocks all write operations)
    pub readonly: bool,
    
    // Warning message for modal dialog
    pub warning_message: Option<String>,
    
    // Custom endpoint URL (for LocalStack, etc.)
    pub endpoint_url: Option<String>,
    
    // SSO login state
    pub sso_state: Option<SsoLoginState>,
    
    // Pagination state
    pub pagination: PaginationState,
    
    // Log tail state
    pub log_tail_state: Option<LogTailState>,
}

/// Pagination state for resource listings
#[derive(Debug, Clone)]
pub struct PaginationState {
    /// Token for fetching next page (None if no more pages)
    pub next_token: Option<String>,
    /// Stack of previous page tokens for going back
    pub token_stack: Vec<Option<String>>,
    /// Current page number (1-indexed for display)
    pub current_page: usize,
    /// Whether there are more pages available
    pub has_more: bool,
}

impl Default for PaginationState {
    fn default() -> Self {
        Self {
            next_token: None,
            token_stack: Vec::new(),
            current_page: 1,
            has_more: false,
        }
    }
}

/// SSO Login dialog state
#[derive(Debug, Clone)]
pub enum SsoLoginState {
    /// Prompt to start login
    Prompt {
        profile: String,
        sso_session: String,
    },
    /// Waiting for browser auth
    WaitingForAuth {
        profile: String,
        user_code: String,
        verification_uri: String,
        #[allow(dead_code)]
        device_code: String,
        #[allow(dead_code)]
        interval: u64,
        #[allow(dead_code)]
        sso_region: String,
    },
    /// Login succeeded - contains profile to switch to
    Success {
        profile: String,
    },
    /// Login failed
    Failed {
        error: String,
    },
}

/// Result of profile switch attempt
#[derive(Debug, Clone)]
pub enum ProfileSwitchResult {
    /// Profile switched successfully
    Success,
    /// SSO login required for this profile
    SsoRequired { profile: String, sso_session: String },
}

/// A single log event from CloudWatch
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub timestamp: i64,
    pub message: String,
}

/// State for log tailing mode
#[derive(Debug, Clone)]
pub struct LogTailState {
    /// Log group name
    pub log_group: String,
    /// Log stream name
    pub log_stream: String,
    /// Collected log events (max 1000)
    pub events: Vec<LogEvent>,
    /// Scroll position in the log view
    pub scroll: usize,
    /// Token for fetching next batch of events
    pub next_forward_token: Option<String>,
    /// Whether to auto-scroll to bottom on new events
    pub auto_scroll: bool,
    /// Whether polling is paused
    pub paused: bool,
    /// Last time we polled for new events
    pub last_poll: std::time::Instant,
    /// Error message if polling failed
    pub error: Option<String>,
}

impl App {
    /// Create App from pre-initialized components (used with splash screen)
    #[allow(clippy::too_many_arguments)]
    pub fn from_initialized(
        clients: AwsClients,
        profile: String,
        region: String,
        available_profiles: Vec<String>,
        available_regions: Vec<String>,
        initial_items: Vec<Value>,
        config: Config,
        readonly: bool,
        endpoint_url: Option<String>,
    ) -> Self {
        let filtered_items = initial_items.clone();
        
        Self {
            clients,
            current_resource_key: "ec2-instances".to_string(),
            items: initial_items,
            filtered_items,
            selected: 0,
            mode: Mode::Normal,
            filter_text: String::new(),
            filter_active: false,
            parent_context: None,
            navigation_stack: Vec::new(),
            command_text: String::new(),
            command_suggestions: Vec::new(),
            command_suggestion_selected: 0,
            command_preview: None,
            profile,
            region,
            available_profiles,
            available_regions,
            profiles_selected: 0,
            regions_selected: 0,
            pending_action: None,
            loading: false,
            error_message: None,
            describe_scroll: 0,
            describe_data: None,
            last_refresh: std::time::Instant::now(),
            config,
            last_key_press: None,
            readonly,
            warning_message: None,
            endpoint_url,
            sso_state: None,
            pagination: PaginationState::default(),
            log_tail_state: None,
        }
    }
    
    /// Check if auto-refresh is needed (every 5 seconds)
    pub fn needs_refresh(&self) -> bool {
        // Only auto-refresh in Normal mode, not when in dialogs/command/etc.
        if self.mode != Mode::Normal {
            return false;
        }
        // Don't refresh while already loading
        if self.loading {
            return false;
        }
        self.last_refresh.elapsed() >= std::time::Duration::from_secs(5)
    }
    
    /// Reset refresh timer
    pub fn mark_refreshed(&mut self) {
        self.last_refresh = std::time::Instant::now();
    }

    // =========================================================================
    // Resource Definition Access
    // =========================================================================

    /// Get current resource definition
    pub fn current_resource(&self) -> Option<&'static ResourceDef> {
        get_resource(&self.current_resource_key)
    }

    /// Get available commands for autocomplete
    pub fn get_available_commands(&self) -> Vec<String> {
        let mut commands: Vec<String> = get_all_resource_keys()
            .iter()
            .map(|s| s.to_string())
            .collect();
        
        // Add profiles and regions commands
        commands.push("profiles".to_string());
        commands.push("regions".to_string());
        
        commands.sort();
        commands
    }

    // =========================================================================
    // Data Fetching
    // =========================================================================

    /// Fetch data for current resource (first page or current page based on pagination state)
    pub async fn refresh_current(&mut self) -> Result<()> {
        // Fetch the current page (uses pagination.next_token if set by next_page/prev_page)
        self.fetch_page(self.pagination.next_token.clone()).await
    }
    
    /// Fetch a specific page of resources
    async fn fetch_page(&mut self, page_token: Option<String>) -> Result<()> {
        if self.current_resource().is_none() {
            self.error_message = Some(format!("Unknown resource: {}", self.current_resource_key));
            return Ok(());
        }

        self.loading = true;
        self.error_message = None;

        // Build filters from parent context
        let filters = self.build_filters_from_context();
        
        // Use paginated fetch - returns only one page of results
        match fetch_resources_paginated(
            &self.current_resource_key, 
            &self.clients, 
            &filters,
            page_token.as_deref(),
        ).await {
            Ok(result) => {
                // Preserve selection if possible
                let prev_selected = self.selected;
                self.items = result.items;
                self.apply_filter();
                
                // Update pagination state
                self.pagination.has_more = result.next_token.is_some();
                self.pagination.next_token = result.next_token;
                
                // Try to keep the same selection index
                if prev_selected < self.filtered_items.len() {
                    self.selected = prev_selected;
                } else {
                    self.selected = 0;
                }
            }
            Err(e) => {
                self.error_message = Some(aws::client::format_aws_error(&e));
                // Clear items to prevent mismatch between current_resource_key and stale items
                self.items.clear();
                self.filtered_items.clear();
                self.selected = 0;
                self.pagination = PaginationState::default();
            }
        }
        
        self.loading = false;
        self.mark_refreshed();
        Ok(())
    }
    
    /// Fetch next page of resources
    pub async fn next_page(&mut self) -> Result<()> {
        if !self.pagination.has_more {
            return Ok(());
        }
        
        // Save current token to stack for going back
        let current_token = self.pagination.next_token.clone();
        self.pagination.token_stack.push(current_token.clone());
        self.pagination.current_page += 1;
        
        // Fetch next page
        self.fetch_page(current_token).await
    }
    
    /// Fetch previous page of resources
    pub async fn prev_page(&mut self) -> Result<()> {
        if self.pagination.current_page <= 1 {
            return Ok(());
        }
        
        // Pop the previous token from stack
        self.pagination.token_stack.pop(); // Remove current page's token
        let prev_token = self.pagination.token_stack.pop().flatten(); // Get previous page's token
        self.pagination.current_page -= 1;
        
        // Fetch previous page
        self.fetch_page(prev_token).await
    }
    
    /// Reset pagination state (call when navigating to new resource)
    pub fn reset_pagination(&mut self) {
        self.pagination = PaginationState::default();
    }

    /// Build AWS filters from parent context
    /// For S3, this collects both bucket_names and prefix from navigation stack
    fn build_filters_from_context(&self) -> Vec<ResourceFilter> {
        let Some(parent) = &self.parent_context else {
            return Vec::new();
        };
        
        let Some(_resource) = self.current_resource() else {
            return Vec::new();
        };
        
        let mut filters = Vec::new();
        
        // For S3 objects, we need to collect filters from entire navigation stack
        // to preserve bucket_names while adding prefix
        if self.current_resource_key == "s3-objects" {
            // First, check navigation stack for bucket_names (from s3-buckets -> s3-objects)
            for ctx in &self.navigation_stack {
                if ctx.resource_key == "s3-buckets" {
                    if let Some(parent_resource) = get_resource(&ctx.resource_key) {
                        for sub in &parent_resource.sub_resources {
                            if sub.resource_key == "s3-objects" {
                                let bucket_name = extract_json_value(&ctx.item, &sub.parent_id_field);
                                if bucket_name != "-" {
                                    filters.push(ResourceFilter::new(&sub.filter_param, vec![bucket_name]));
                                }
                            }
                        }
                    }
                }
            }
            
            // If parent is s3-buckets, get bucket_names from it
            if parent.resource_key == "s3-buckets" {
                if let Some(parent_resource) = get_resource(&parent.resource_key) {
                    for sub in &parent_resource.sub_resources {
                        if sub.resource_key == "s3-objects" {
                            let bucket_name = extract_json_value(&parent.item, &sub.parent_id_field);
                            if bucket_name != "-" {
                                filters.push(ResourceFilter::new(&sub.filter_param, vec![bucket_name]));
                            }
                        }
                    }
                }
            }
            
            // If parent is s3-objects (folder navigation), get prefix from it
            if parent.resource_key == "s3-objects" {
                // Check if selected item is a folder
                let is_folder = parent.item.get("IsFolder")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                
                if is_folder {
                    let prefix = extract_json_value(&parent.item, "Key");
                    if prefix != "-" {
                        filters.push(ResourceFilter::new("prefix", vec![prefix]));
                    }
                }
            }
            
            return filters;
        }
        
        // Default behavior for other resources
        if let Some(parent_resource) = get_resource(&parent.resource_key) {
            for sub in &parent_resource.sub_resources {
                if sub.resource_key == self.current_resource_key {
                    // Extract parent ID value
                    let parent_id = extract_json_value(&parent.item, &sub.parent_id_field);
                    if parent_id != "-" {
                        return vec![ResourceFilter::new(&sub.filter_param, vec![parent_id])];
                    }
                }
            }
        }
        
        Vec::new()
    }

    // =========================================================================
    // Filtering
    // =========================================================================

    /// Apply text filter to items
    pub fn apply_filter(&mut self) {
        let filter = self.filter_text.to_lowercase();

        if filter.is_empty() {
            self.filtered_items = self.items.clone();
        } else {
            let resource = self.current_resource();
            self.filtered_items = self
                .items
                .iter()
                .filter(|item| {
                    // Search in name field and id field
                    if let Some(res) = resource {
                        let name = extract_json_value(item, &res.name_field).to_lowercase();
                        let id = extract_json_value(item, &res.id_field).to_lowercase();
                        name.contains(&filter) || id.contains(&filter)
                    } else {
                        // Fallback: search in JSON string
                        item.to_string().to_lowercase().contains(&filter)
                    }
                })
                .cloned()
                .collect();
        }

        // Adjust selection
        if self.selected >= self.filtered_items.len() && !self.filtered_items.is_empty() {
            self.selected = self.filtered_items.len() - 1;
        }
    }

    pub fn toggle_filter(&mut self) {
        self.filter_active = !self.filter_active;
    }

    pub fn clear_filter(&mut self) {
        self.filter_text.clear();
        self.filter_active = false;
        self.apply_filter();
    }

    // =========================================================================
    // Navigation
    // =========================================================================

    #[allow(dead_code)]
    pub fn current_list_len(&self) -> usize {
        self.filtered_items.len()
    }

    pub fn selected_item(&self) -> Option<&Value> {
        self.filtered_items.get(self.selected)
    }

    pub fn selected_item_json(&self) -> Option<String> {
        // Use describe_data if available (full details), otherwise fall back to list data
        if let Some(ref data) = self.describe_data {
            return Some(serde_json::to_string_pretty(data).unwrap_or_default());
        }
        self.selected_item()
            .map(|item| serde_json::to_string_pretty(item).unwrap_or_default())
    }

    /// Get the number of lines in the describe content
    pub fn describe_line_count(&self) -> usize {
        self.selected_item_json()
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// Clamp describe scroll to valid range
    #[allow(dead_code)]
    pub fn clamp_describe_scroll(&mut self, visible_lines: usize) {
        let total = self.describe_line_count();
        let max_scroll = total.saturating_sub(visible_lines);
        self.describe_scroll = self.describe_scroll.min(max_scroll);
    }

    /// Scroll describe view to bottom
    pub fn describe_scroll_to_bottom(&mut self, visible_lines: usize) {
        let total = self.describe_line_count();
        self.describe_scroll = total.saturating_sub(visible_lines);
    }

    pub fn next(&mut self) {
        match self.mode {
            Mode::Profiles => {
                if !self.available_profiles.is_empty() {
                    self.profiles_selected = (self.profiles_selected + 1).min(self.available_profiles.len() - 1);
                }
            }
            Mode::Regions => {
                if !self.available_regions.is_empty() {
                    self.regions_selected = (self.regions_selected + 1).min(self.available_regions.len() - 1);
                }
            }
            _ => {
                if !self.filtered_items.is_empty() {
                    self.selected = (self.selected + 1).min(self.filtered_items.len() - 1);
                }
            }
        }
    }

    pub fn previous(&mut self) {
        match self.mode {
            Mode::Profiles => {
                self.profiles_selected = self.profiles_selected.saturating_sub(1);
            }
            Mode::Regions => {
                self.regions_selected = self.regions_selected.saturating_sub(1);
            }
            _ => {
                self.selected = self.selected.saturating_sub(1);
            }
        }
    }

    pub fn go_to_top(&mut self) {
        match self.mode {
            Mode::Profiles => self.profiles_selected = 0,
            Mode::Regions => self.regions_selected = 0,
            _ => self.selected = 0,
        }
    }

    pub fn go_to_bottom(&mut self) {
        match self.mode {
            Mode::Profiles => {
                if !self.available_profiles.is_empty() {
                    self.profiles_selected = self.available_profiles.len() - 1;
                }
            }
            Mode::Regions => {
                if !self.available_regions.is_empty() {
                    self.regions_selected = self.available_regions.len() - 1;
                }
            }
            _ => {
                if !self.filtered_items.is_empty() {
                    self.selected = self.filtered_items.len() - 1;
                }
            }
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        match self.mode {
            Mode::Profiles => {
                if !self.available_profiles.is_empty() {
                    self.profiles_selected = (self.profiles_selected + page_size).min(self.available_profiles.len() - 1);
                }
            }
            Mode::Regions => {
                if !self.available_regions.is_empty() {
                    self.regions_selected = (self.regions_selected + page_size).min(self.available_regions.len() - 1);
                }
            }
            _ => {
                if !self.filtered_items.is_empty() {
                    self.selected = (self.selected + page_size).min(self.filtered_items.len() - 1);
                }
            }
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        match self.mode {
            Mode::Profiles => {
                self.profiles_selected = self.profiles_selected.saturating_sub(page_size);
            }
            Mode::Regions => {
                self.regions_selected = self.regions_selected.saturating_sub(page_size);
            }
            _ => {
                self.selected = self.selected.saturating_sub(page_size);
            }
        }
    }

    // =========================================================================
    // Mode Transitions
    // =========================================================================

    pub fn enter_command_mode(&mut self) {
        self.mode = Mode::Command;
        self.command_text.clear();
        self.command_suggestions = self.get_available_commands();
        self.command_suggestion_selected = 0;
        self.command_preview = None;
    }

    pub fn update_command_suggestions(&mut self) {
        let input = self.command_text.to_lowercase();
        let all_commands = self.get_available_commands();
        
        if input.is_empty() {
            self.command_suggestions = all_commands;
        } else {
            self.command_suggestions = all_commands
                .into_iter()
                .filter(|cmd| cmd.contains(&input))
                .collect();
        }
        
        if self.command_suggestion_selected >= self.command_suggestions.len() {
            self.command_suggestion_selected = 0;
        }
        
        // Update preview to show current selection
        self.update_preview();
    }
    
    fn update_preview(&mut self) {
        if self.command_suggestions.is_empty() {
            self.command_preview = None;
        } else {
            self.command_preview = self.command_suggestions
                .get(self.command_suggestion_selected)
                .cloned();
        }
    }

    pub fn next_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            self.command_suggestion_selected = 
                (self.command_suggestion_selected + 1) % self.command_suggestions.len();
            // Update preview (ghost text) without changing command_text
            self.update_preview();
        }
    }

    pub fn prev_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            if self.command_suggestion_selected == 0 {
                self.command_suggestion_selected = self.command_suggestions.len() - 1;
            } else {
                self.command_suggestion_selected -= 1;
            }
            // Update preview (ghost text) without changing command_text
            self.update_preview();
        }
    }

    pub fn apply_suggestion(&mut self) {
        // Apply the preview to command_text (on Tab/Right)
        if let Some(preview) = &self.command_preview {
            self.command_text = preview.clone();
            self.update_command_suggestions();
        }
    }

    pub fn enter_help_mode(&mut self) {
        self.mode = Mode::Help;
    }

    pub async fn enter_describe_mode(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        
        self.mode = Mode::Describe;
        self.describe_scroll = 0;
        self.describe_data = None;
        
        // Get the selected item's ID
        if let Some(item) = self.selected_item() {
            if let Some(resource_def) = self.current_resource() {
                let id = crate::resource::extract_json_value(item, &resource_def.id_field);
                if id != "-" && !id.is_empty() {
                    // Fetch full details
                    match crate::resource::describe_resource(
                        &self.current_resource_key,
                        &self.clients,
                        &id,
                    ).await {
                        Ok(data) => {
                            self.describe_data = Some(data);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch describe data: {}", e);
                            // Fall back to list data
                            self.describe_data = Some(item.clone());
                        }
                    }
                }
            }
        }
    }

    /// Enter confirmation mode for an action
    pub fn enter_confirm_mode(&mut self, pending: PendingAction) {
        self.pending_action = Some(pending);
        self.mode = Mode::Confirm;
    }
    
    /// Show a warning modal with OK button
    pub fn show_warning(&mut self, message: &str) {
        self.warning_message = Some(message.to_string());
        self.mode = Mode::Warning;
    }
    
    /// Enter SSO login mode to prompt for browser authentication
    pub fn enter_sso_login_mode(&mut self, profile: &str, sso_session: &str) {
        self.sso_state = Some(SsoLoginState::Prompt {
            profile: profile.to_string(),
            sso_session: sso_session.to_string(),
        });
        self.mode = Mode::SsoLogin;
    }
    
    /// Create a pending action from an ActionDef
    pub fn create_pending_action(&self, action: &crate::resource::ActionDef, resource_id: &str) -> Option<PendingAction> {
        let config = action.get_confirm_config()?;
        let resource_name = self.selected_item()
            .and_then(|item| {
                if let Some(resource_def) = self.current_resource() {
                    let name = crate::resource::extract_json_value(item, &resource_def.name_field);
                    if name != "-" && !name.is_empty() {
                        return Some(name);
                    }
                }
                None
            })
            .unwrap_or_else(|| resource_id.to_string());
        
        let message = config.message.unwrap_or_else(|| action.display_name.clone());
        let default_no = !config.default_yes;
        
        Some(PendingAction {
            service: self.current_resource()?.service.clone(),
            sdk_method: action.sdk_method.clone(),
            resource_id: resource_id.to_string(),
            message: format!("{} '{}'?", message, resource_name),
            default_no,
            destructive: config.destructive,
            selected_yes: config.default_yes, // Start with default selection
        })
    }

    pub fn enter_profiles_mode(&mut self) {
        self.profiles_selected = self
            .available_profiles
            .iter()
            .position(|p| p == &self.profile)
            .unwrap_or(0);
        self.mode = Mode::Profiles;
    }

    pub fn enter_regions_mode(&mut self) {
        self.regions_selected = self
            .available_regions
            .iter()
            .position(|r| r == &self.region)
            .unwrap_or(0);
        self.mode = Mode::Regions;
    }

    pub fn exit_mode(&mut self) {
        self.mode = Mode::Normal;
        self.pending_action = None;
        self.describe_data = None;  // Clear describe data when exiting
    }

    // =========================================================================
    // Resource Navigation
    // =========================================================================

    /// Navigate to a resource (top-level)
    pub async fn navigate_to_resource(&mut self, resource_key: &str) -> Result<()> {
        if get_resource(resource_key).is_none() {
            self.error_message = Some(format!("Unknown resource: {}", resource_key));
            return Ok(());
        }
        
        // Clear parent context when navigating to top-level resource
        self.parent_context = None;
        self.navigation_stack.clear();
        self.current_resource_key = resource_key.to_string();
        self.selected = 0;
        self.filter_text.clear();
        self.filter_active = false;
        self.mode = Mode::Normal;
        
        // Reset pagination for new resource
        self.reset_pagination();
        
        self.refresh_current().await?;
        Ok(())
    }

    /// Navigate to sub-resource with parent context
    pub async fn navigate_to_sub_resource(&mut self, sub_resource_key: &str) -> Result<()> {
        let Some(selected_item) = self.selected_item().cloned() else {
            return Ok(());
        };
        
        let Some(current_resource) = self.current_resource() else {
            return Ok(());
        };
        
        // Verify this is a valid sub-resource
        let is_valid = current_resource
            .sub_resources
            .iter()
            .any(|s| s.resource_key == sub_resource_key);
        
        if !is_valid {
            self.error_message = Some(format!(
                "{} is not a sub-resource of {}",
                sub_resource_key, self.current_resource_key
            ));
            return Ok(());
        }
        
        // Special handling for S3 folder navigation
        // Only allow navigating into folders, not files
        if self.current_resource_key == "s3-objects" && sub_resource_key == "s3-objects" {
            let is_folder = selected_item.get("IsFolder")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            
            if !is_folder {
                // Don't navigate into files - could show a message or do nothing
                return Ok(());
            }
        }
        
        // Get display name for parent
        let display_name = extract_json_value(&selected_item, &current_resource.name_field);
        let id = extract_json_value(&selected_item, &current_resource.id_field);
        let display = if display_name != "-" { display_name } else { id };
        
        // Push current context to stack
        if let Some(ctx) = self.parent_context.take() {
            self.navigation_stack.push(ctx);
        }
        
        // Set new parent context
        self.parent_context = Some(ParentContext {
            resource_key: self.current_resource_key.clone(),
            item: selected_item,
            display_name: display,
        });
        
        // Navigate
        self.current_resource_key = sub_resource_key.to_string();
        self.selected = 0;
        self.filter_text.clear();
        self.filter_active = false;
        
        // Reset pagination for new resource
        self.reset_pagination();
        
        self.refresh_current().await?;
        Ok(())
    }

    /// Navigate back to parent resource
    pub async fn navigate_back(&mut self) -> Result<()> {
        if let Some(parent) = self.parent_context.take() {
            // Pop from navigation stack if available
            self.parent_context = self.navigation_stack.pop();
            
            // Navigate to parent resource
            self.current_resource_key = parent.resource_key;
            self.selected = 0;
            self.filter_text.clear();
            self.filter_active = false;
            
            // Reset pagination for parent resource
            self.reset_pagination();
            
            self.refresh_current().await?;
        }
        Ok(())
    }

    /// Get breadcrumb path
    pub fn get_breadcrumb(&self) -> Vec<String> {
        let mut path = Vec::new();
        
        for ctx in &self.navigation_stack {
            path.push(format!("{}:{}", ctx.resource_key, ctx.display_name));
        }
        
        if let Some(ctx) = &self.parent_context {
            path.push(format!("{}:{}", ctx.resource_key, ctx.display_name));
        }
        
        path.push(self.current_resource_key.clone());
        path
    }

    // =========================================================================
    // EC2 Actions (using SDK dispatcher)
    // =========================================================================
    // Profile/Region Switching
    // =========================================================================

    pub async fn switch_region(&mut self, region: &str) -> Result<()> {
        let actual_region = self.clients.switch_region(&self.profile, region).await?;
        self.region = actual_region.clone();
        
        // Save to config (ignore errors - don't fail region switch if config save fails)
        let _ = self.config.set_region(&actual_region);
        
        Ok(())
    }

    pub async fn switch_profile(&mut self, profile: &str) -> Result<()> {
        let (new_clients, actual_region) = AwsClients::new(profile, &self.region, self.endpoint_url.clone()).await?;
        self.clients = new_clients;
        self.profile = profile.to_string();
        self.region = actual_region.clone();
        
        // Save to config (ignore errors - don't fail profile switch if config save fails)
        let _ = self.config.set_profile(profile);
        let _ = self.config.set_region(&actual_region);
        
        Ok(())
    }
    
    /// Switch profile with SSO check - returns SsoRequired if SSO login is needed
    pub async fn switch_profile_with_sso_check(&mut self, profile: &str) -> Result<ProfileSwitchResult> {
        use crate::aws::client::ClientResult;
        
        match AwsClients::new_with_sso_check(profile, &self.region, self.endpoint_url.clone()).await? {
            ClientResult::Ok(new_clients, actual_region) => {
                self.clients = new_clients;
                self.profile = profile.to_string();
                self.region = actual_region.clone();
                
                // Save to config
                let _ = self.config.set_profile(profile);
                let _ = self.config.set_region(&actual_region);
                
                Ok(ProfileSwitchResult::Success)
            }
            ClientResult::SsoLoginRequired { profile, sso_session, .. } => {
                Ok(ProfileSwitchResult::SsoRequired { profile, sso_session })
            }
        }
    }

    /// Select profile - returns true if SSO login is required
    pub async fn select_profile(&mut self) -> Result<bool> {
        if let Some(profile) = self.available_profiles.get(self.profiles_selected) {
            let profile = profile.clone();
            match self.switch_profile_with_sso_check(&profile).await? {
                ProfileSwitchResult::Success => {
                    self.refresh_current().await?;
                    self.exit_mode();
                    Ok(false)
                }
                ProfileSwitchResult::SsoRequired { profile, sso_session } => {
                    // Enter SSO login mode
                    self.enter_sso_login_mode(&profile, &sso_session);
                    Ok(true)
                }
            }
        } else {
            self.exit_mode();
            Ok(false)
        }
    }

    pub async fn select_region(&mut self) -> Result<()> {
        if let Some(region) = self.available_regions.get(self.regions_selected) {
            let region = region.clone();
            self.switch_region(&region).await?;
            self.refresh_current().await?;
        }
        self.exit_mode();
        Ok(())
    }

    // =========================================================================
    // Command Execution
    // =========================================================================

    pub async fn execute_command(&mut self) -> Result<bool> {
        // Use preview if user navigated to a suggestion, otherwise use typed text
        let command_text = if self.command_text.is_empty() {
            self.command_preview.clone().unwrap_or_default()
        } else if let Some(preview) = &self.command_preview {
            // If preview matches what would be completed, use preview
            if preview.contains(&self.command_text) {
                preview.clone()
            } else {
                self.command_text.clone()
            }
        } else {
            self.command_text.clone()
        };
        
        let parts: Vec<&str> = command_text.split_whitespace().collect();
        
        if parts.is_empty() {
            return Ok(false);
        }

        let cmd = parts[0];

        match cmd {
            "q" | "quit" => return Ok(true),
            "back" => {
                self.navigate_back().await?;
            }
            "profiles" => {
                self.enter_profiles_mode();
            }
            "regions" => {
                self.enter_regions_mode();
            }
            "region" if parts.len() > 1 => {
                self.switch_region(parts[1]).await?;
                self.refresh_current().await?;
            }
            "profile" if parts.len() > 1 => {
                self.switch_profile(parts[1]).await?;
                self.refresh_current().await?;
            }
            _ => {
                // Check if it's a known resource
                if get_resource(cmd).is_some() {
                    // Check if it's a sub-resource of current
                    if let Some(resource) = self.current_resource() {
                        let is_sub = resource.sub_resources.iter().any(|s| s.resource_key == cmd);
                        if is_sub && self.selected_item().is_some() {
                            self.navigate_to_sub_resource(cmd).await?;
                        } else {
                            self.navigate_to_resource(cmd).await?;
                        }
                    } else {
                        self.navigate_to_resource(cmd).await?;
                    }
                } else {
                    self.error_message = Some(format!("Unknown command: {}", cmd));
                }
            }
        }

        Ok(false)
    }

    // =========================================================================
    // Log Tail Mode
    // =========================================================================

    /// Enter log tail mode for the selected log stream
    pub async fn enter_log_tail_mode(&mut self) -> Result<()> {
        // Get the selected log stream item
        let Some(item) = self.selected_item().cloned() else {
            return Ok(());
        };

        // Extract log group and stream names
        let log_group = extract_json_value(&item, "logGroupName");
        let log_stream = extract_json_value(&item, "logStreamName");

        if log_group == "-" || log_stream == "-" {
            self.error_message = Some("Could not get log group/stream name".to_string());
            return Ok(());
        }

        // Initialize log tail state
        self.log_tail_state = Some(LogTailState {
            log_group: log_group.clone(),
            log_stream: log_stream.clone(),
            events: Vec::new(),
            scroll: 0,
            next_forward_token: None,
            auto_scroll: true,
            paused: false,
            last_poll: std::time::Instant::now(),
            error: None,
        });

        self.mode = Mode::LogTail;

        // Fetch initial log events
        self.poll_log_events().await?;

        Ok(())
    }

    /// Poll for new log events
    pub async fn poll_log_events(&mut self) -> Result<()> {
        let Some(ref mut state) = self.log_tail_state else {
            return Ok(());
        };

        if state.paused {
            return Ok(());
        }

        // Build params for get_log_events
        let mut params = serde_json::json!({
            "log_group_name": [state.log_group.clone()],
            "log_stream_name": [state.log_stream.clone()],
        });

        if let Some(ref token) = state.next_forward_token {
            params["next_forward_token"] = serde_json::json!(token);
        }

        // Call the SDK
        match crate::resource::sdk_dispatch::invoke_sdk(
            "cloudwatchlogs",
            "get_log_events",
            &self.clients,
            &params,
        ).await {
            Ok(response) => {
                state.error = None;
                
                // Extract events
                if let Some(events) = response.get("events").and_then(|v| v.as_array()) {
                    for event in events {
                        let timestamp = event.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
                        let message = event.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        
                        state.events.push(LogEvent { timestamp, message });
                    }
                    
                    // Keep only last 1000 events
                    if state.events.len() > 1000 {
                        let drain_count = state.events.len() - 1000;
                        state.events.drain(0..drain_count);
                    }
                }

                // Update next forward token
                if let Some(token) = response.get("nextForwardToken").and_then(|v| v.as_str()) {
                    state.next_forward_token = Some(token.to_string());
                }

                // Auto-scroll to bottom if enabled
                if state.auto_scroll && !state.events.is_empty() {
                    state.scroll = state.events.len().saturating_sub(1);
                }
            }
            Err(e) => {
                state.error = Some(format!("Failed to fetch logs: {}", e));
            }
        }

        state.last_poll = std::time::Instant::now();
        Ok(())
    }

    /// Toggle pause state for log tailing
    pub fn toggle_log_tail_pause(&mut self) {
        if let Some(ref mut state) = self.log_tail_state {
            state.paused = !state.paused;
        }
    }

    /// Scroll log tail view up
    pub fn log_tail_scroll_up(&mut self, amount: usize) {
        if let Some(ref mut state) = self.log_tail_state {
            state.scroll = state.scroll.saturating_sub(amount);
            state.auto_scroll = false;
        }
    }

    /// Scroll log tail view down
    pub fn log_tail_scroll_down(&mut self, amount: usize) {
        if let Some(ref mut state) = self.log_tail_state {
            let max_scroll = state.events.len().saturating_sub(1);
            state.scroll = (state.scroll + amount).min(max_scroll);
        }
    }

    /// Scroll log tail view to top
    pub fn log_tail_scroll_to_top(&mut self) {
        if let Some(ref mut state) = self.log_tail_state {
            state.scroll = 0;
            state.auto_scroll = false;
        }
    }

    /// Scroll log tail view to bottom and enable auto-scroll
    pub fn log_tail_scroll_to_bottom(&mut self) {
        if let Some(ref mut state) = self.log_tail_state {
            state.scroll = state.events.len().saturating_sub(1);
            state.auto_scroll = true;
        }
    }

    /// Exit log tail mode
    pub fn exit_log_tail_mode(&mut self) {
        self.log_tail_state = None;
        self.mode = Mode::Normal;
    }
}
