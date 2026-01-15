//! Configuration management for taws
//!
//! Stores user preferences in ~/.config/taws/config.yaml (XDG compliant)
//! Falls back to ~/.taws/config.yaml if XDG dirs not available

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, warn};

/// User configuration stored on disk
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Last used AWS profile
    #[serde(default)]
    pub profile: Option<String>,

    /// Last used AWS region
    #[serde(default)]
    pub region: Option<String>,

    /// Last viewed resource type
    #[serde(default)]
    pub last_resource: Option<String>,

    /// Recently used regions (most recent first, max 6)
    #[serde(default)]
    pub recently_used_regions: Vec<String>,
}

impl Config {
    /// Load config from disk, or return default if not found
    pub fn load() -> Self {
        let path = Self::config_path();
        debug!("Loading config from {:?}", path);

        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(contents) => match serde_yaml::from_str(&contents) {
                    Ok(config) => {
                        debug!("Config loaded successfully: {:?}", config);
                        return config;
                    }
                    Err(e) => {
                        warn!("Failed to parse config: {}", e);
                    }
                },
                Err(e) => {
                    warn!("Failed to read config: {}", e);
                }
            }
        } else {
            debug!("Config file does not exist, using defaults");
        }

        Self::default()
    }

    /// Save config to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        debug!("Saving config to {:?}", path);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            debug!("Creating parent directory: {:?}", parent);
            fs::create_dir_all(parent)?;
        }

        let contents = serde_yaml::to_string(self)?;
        fs::write(&path, contents)?;
        debug!("Config saved successfully: {:?}", self);

        Ok(())
    }

    /// Get the config file path
    /// Uses XDG config directory if available, otherwise ~/.taws/
    fn config_path() -> PathBuf {
        // Try XDG config dir first (e.g., ~/.config/taws/config.yaml)
        if let Some(config_dir) = dirs::config_dir() {
            return config_dir.join("taws").join("config.yaml");
        }

        // Fallback to home directory
        if let Some(home) = dirs::home_dir() {
            return home.join(".taws").join("config.yaml");
        }

        // Last resort: current directory
        PathBuf::from(".taws").join("config.yaml")
    }

    /// Update profile and save
    pub fn set_profile(&mut self, profile: &str) -> Result<()> {
        debug!("Setting profile to: {}", profile);
        self.profile = Some(profile.to_string());
        self.save()
    }

    /// Update region and save
    pub fn set_region(&mut self, region: &str) -> Result<()> {
        debug!("Setting region to: {}", region);
        self.region = Some(region.to_string());
        self.add_recent_region(region);
        self.save()
    }

    /// Add region to recently used list (most recent first, max 6)
    fn add_recent_region(&mut self, region: &str) {
        // Remove if already exists
        self.recently_used_regions.retain(|r| r != region);
        // Add to front
        self.recently_used_regions.insert(0, region.to_string());
        // Keep max 6
        self.recently_used_regions.truncate(6);
    }

    /// Get recently used regions for display (returns up to 6)
    pub fn get_recent_regions(&self) -> Vec<String> {
        self.recently_used_regions.clone()
    }

    /// Update last resource and save
    #[allow(dead_code)]
    pub fn set_last_resource(&mut self, resource: &str) -> Result<()> {
        self.last_resource = Some(resource.to_string());
        self.save()
    }

    /// Get effective profile (config -> env -> default)
    pub fn effective_profile(&self) -> String {
        // Priority: 1. Environment variable, 2. Config file, 3. Default
        std::env::var("AWS_PROFILE")
            .ok()
            .or_else(|| self.profile.clone())
            .unwrap_or_else(|| "default".to_string())
    }

    /// Get effective region (config -> env -> default)
    pub fn effective_region(&self) -> String {
        // Priority: 1. Environment variable, 2. Config file, 3. Default
        std::env::var("AWS_REGION")
            .ok()
            .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
            .or_else(|| self.region.clone())
            .unwrap_or_else(|| "us-east-1".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.profile.is_none());
        assert!(config.region.is_none());
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = Config {
            profile: Some("my-profile".to_string()),
            region: Some("eu-west-1".to_string()),
            last_resource: Some("ec2-instances".to_string()),
            recently_used_regions: vec!["eu-west-1".to_string(), "us-east-1".to_string()],
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.profile, config.profile);
        assert_eq!(parsed.region, config.region);
        assert_eq!(parsed.last_resource, config.last_resource);
        assert_eq!(parsed.recently_used_regions, config.recently_used_regions);
    }

    #[test]
    fn test_add_recent_region() {
        let mut config = Config::default();

        config.add_recent_region("us-east-1");
        assert_eq!(config.recently_used_regions, vec!["us-east-1"]);

        config.add_recent_region("eu-west-1");
        assert_eq!(config.recently_used_regions, vec!["eu-west-1", "us-east-1"]);

        // Adding existing region moves it to front
        config.add_recent_region("us-east-1");
        assert_eq!(config.recently_used_regions, vec!["us-east-1", "eu-west-1"]);

        // Max 6 regions
        config.add_recent_region("r1");
        config.add_recent_region("r2");
        config.add_recent_region("r3");
        config.add_recent_region("r4");
        config.add_recent_region("r5");
        assert_eq!(config.recently_used_regions.len(), 6);
        assert_eq!(config.recently_used_regions[0], "r5");
    }
}
