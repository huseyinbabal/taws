//! Configuration management for taws
//!
//! Stores user preferences in ~/.config/taws/config.yaml (XDG compliant)
//! Falls back to ~/.taws/config.yaml if XDG dirs not available

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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

    /// Preferred region shortcuts for the header (<=5 entries)
    #[serde(default)]
    pub region_shortcuts: Vec<String>,
}

impl Config {
    /// Load config from disk, or return default if not found
    pub fn load() -> Self {
        let path = Self::config_path();
        
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(contents) => {
                    match serde_yaml::from_str(&contents) {
                        Ok(config) => return config,
                        Err(e) => {
                            eprintln!("Warning: Failed to parse config: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to read config: {}", e);
                }
            }
        }
        
        Self::default()
    }
    
    /// Save config to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let contents = serde_yaml::to_string(self)?;
        fs::write(&path, contents)?;
        
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
        self.profile = Some(profile.to_string());
        self.save()
    }
    
    /// Update region and save
    pub fn set_region(&mut self, region: &str) -> Result<()> {
        self.region = Some(region.to_string());
        self.save()
    }
    
    /// Update last resource and save
    #[allow(dead_code)]
    pub fn set_last_resource(&mut self, resource: &str) -> Result<()> {
        self.last_resource = Some(resource.to_string());
        self.save()
    }

    /// Update preferred region shortcuts and save
    pub fn set_region_shortcuts(&mut self, shortcuts: &[String]) -> Result<()> {
        self.region_shortcuts = shortcuts.to_vec();
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
            region_shortcuts: vec!["us-east-1".to_string()],
        };
        
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        
        assert_eq!(parsed.profile, config.profile);
        assert_eq!(parsed.region, config.region);
        assert_eq!(parsed.last_resource, config.last_resource);
        assert_eq!(parsed.region_shortcuts, config.region_shortcuts);
    }
}
