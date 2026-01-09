use anyhow::Result;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tracing::warn;

const FALLBACK_REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "af-south-1",
    "ap-east-1",
    "ap-south-1",
    "ap-south-2",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-southeast-4",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ca-central-1",
    "eu-central-1",
    "eu-central-2",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-south-1",
    "eu-south-2",
    "eu-north-1",
    "me-south-1",
    "me-central-1",
    "sa-east-1",
];

/// List all AWS profiles from ~/.aws/credentials and ~/.aws/config
pub fn list_profiles() -> Result<Vec<String>> {
    let mut profiles = HashSet::new();

    // Always include default
    profiles.insert("default".to_string());

    // Read from ~/.aws/credentials
    if let Some(creds_path) = get_aws_credentials_path() {
        if let Ok(content) = fs::read_to_string(&creds_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    let profile = line[1..line.len() - 1].to_string();
                    profiles.insert(profile);
                }
            }
        }
    }

    // Read from ~/.aws/config
    if let Some(config_path) = get_aws_config_path() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    let section = &line[1..line.len() - 1];
                    // Config file uses "profile <name>" format, except for default
                    let profile = if section.starts_with("profile ") {
                        section.strip_prefix("profile ").unwrap().to_string()
                    } else {
                        section.to_string()
                    };
                    profiles.insert(profile);
                }
            }
        }
    }

    let mut profiles: Vec<String> = profiles.into_iter().collect();
    profiles.sort();

    Ok(profiles)
}

/// List common AWS regions
pub fn list_regions() -> Vec<String> {
    match fetch_regions_via_aws_cli() {
        Ok(regions) if !regions.is_empty() => {
            regions
        }
        Ok(_) => {
            warn!("received empty AWS region list, falling back to static list");
            fallback_regions()
        }
        Err(error) => {
            warn!(?error, "failed to fetch AWS regions via CLI, falling back to static list");
            fallback_regions()
        }
    }
}

fn get_aws_credentials_path() -> Option<PathBuf> {
    // Check AWS_SHARED_CREDENTIALS_FILE env var first
    if let Ok(path) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        return Some(PathBuf::from(path));
    }

    // Fall back to ~/.aws/credentials
    dirs::home_dir().map(|h| h.join(".aws").join("credentials"))
}

fn get_aws_config_path() -> Option<PathBuf> {
    // Check AWS_CONFIG_FILE env var first
    if let Ok(path) = std::env::var("AWS_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }

    // Fall back to ~/.aws/config
    dirs::home_dir().map(|h| h.join(".aws").join("config"))
}

fn fallback_regions() -> Vec<String> {
    FALLBACK_REGIONS.iter().map(|region| region.to_string()).collect()
}

fn fetch_regions_via_aws_cli() -> Result<Vec<String>> {
    let output = Command::new("aws")
        .args([
            "--output",
            "json",
            "ec2",
            "describe-regions",
            "--filters",
            "Name=opt-in-status,Values=opt-in-not-required,opted-in",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "aws CLI returned non-zero status ({}): {}",
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8(output.stdout)?;
    let response: DescribeRegionsResponse = serde_json::from_str(&stdout)?;

    let mut regions: Vec<String> = response
        .regions
        .into_iter()
        .filter_map(|region| region.region_name)
        .collect();
    regions.sort();
    regions.dedup();
    Ok(regions)
}

#[derive(Debug, Deserialize)]
struct DescribeRegionsResponse {
    #[serde(default, rename = "Regions")]
    regions: Vec<RegionSummary>,
}

#[derive(Debug, Deserialize)]
struct RegionSummary {
    #[serde(rename = "RegionName")]
    region_name: Option<String>,
}

