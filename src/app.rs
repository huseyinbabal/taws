use crate::aws;
use crate::aws::client::AwsClients;
use crate::config::Config;
use crossterm::event::KeyCode;
use crate::resource::{
    get_resource, get_all_resource_keys, ResourceDef, ResourceFilter, 
    fetch_resources, extract_json_value,
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
}

impl App {
    /// Create App from pre-initialized components (used with splash screen)
    pub fn from_initialized(
        clients: AwsClients,
        profile: String,
        region: String,
        available_profiles: Vec<String>,
        available_regions: Vec<String>,
        initial_items: Vec<Value>,
        config: Config,
        readonly: bool,
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

    /// Fetch data for current resource
    pub async fn refresh_current(&mut self) -> Result<()> {
        if self.current_resource().is_none() {
            self.error_message = Some(format!("Unknown resource: {}", self.current_resource_key));
            return Ok(());
        }

        self.loading = true;
        self.error_message = None;

        // Build filters from parent context
        let filters = self.build_filters_from_context();
        
        // Use the new generic fetch_resources function
        match fetch_resources(&self.current_resource_key, &self.clients, &filters).await {
            Ok(items) => {
                // Preserve selection if possible
                let prev_selected = self.selected;
                self.items = items;
                self.apply_filter();
                // Try to keep the same selection index
                if prev_selected < self.filtered_items.len() {
                    self.selected = prev_selected;
                }
            }
            Err(e) => {
                self.error_message = Some(aws::client::format_aws_error(&e));
                // Clear items to prevent mismatch between current_resource_key and stale items
                self.items.clear();
                self.filtered_items.clear();
                self.selected = 0;
            }
        }
        
        self.loading = false;
        self.mark_refreshed();
        Ok(())
    }

    /// Build AWS filters from parent context
    fn build_filters_from_context(&self) -> Vec<ResourceFilter> {
        let Some(parent) = &self.parent_context else {
            return Vec::new();
        };
        
        let Some(_resource) = self.current_resource() else {
            return Vec::new();
        };
        
        // Find matching sub-resource definition in parent
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
        let (new_clients, actual_region) = AwsClients::new(profile, &self.region).await?;
        self.clients = new_clients;
        self.profile = profile.to_string();
        self.region = actual_region.clone();
        
        // Save to config (ignore errors - don't fail profile switch if config save fails)
        let _ = self.config.set_profile(profile);
        let _ = self.config.set_region(&actual_region);
        
        Ok(())
    }

    pub async fn select_profile(&mut self) -> Result<()> {
        if let Some(profile) = self.available_profiles.get(self.profiles_selected) {
            let profile = profile.clone();
            self.switch_profile(&profile).await?;
            self.refresh_current().await?;
        }
        self.exit_mode();
        Ok(())
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
}
