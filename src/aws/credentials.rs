//! AWS Credentials loading from multiple sources
//!
//! Supports:
//! - Environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN)
//! - AWS profiles (~/.aws/credentials and ~/.aws/config)
//! - AWS SSO (IAM Identity Center) via cached tokens
//! - IAM Role assumption via role_arn and source_profile/credential_source
//! - ECS container credentials (via credential_source = EcsContainer)
//! - IMDSv2 (EC2 instance metadata)

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, trace};

/// Specific error for SSO login required
#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error("SSO login required for profile '{profile}' (session: {sso_session})")]
    SsoLoginRequired {
        profile: String,
        sso_session: String,
    },

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// AWS credentials
#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// Cached IMDS credentials with expiration
struct CachedImdsCredentials {
    credentials: Credentials,
    expiration: Instant,
}

/// Global cache for IMDS credentials
static IMDS_CACHE: OnceLock<std::sync::Mutex<Option<CachedImdsCredentials>>> = OnceLock::new();

/// Global cache for SSO credentials (keyed by profile name)
static SSO_CACHE: OnceLock<std::sync::Mutex<HashMap<String, CachedImdsCredentials>>> =
    OnceLock::new();

/// Global cache for Process credentials (keyed by profile name)
static PROCESS_CACHE: OnceLock<std::sync::Mutex<HashMap<String, CachedImdsCredentials>>> =
    OnceLock::new();

/// Global cache for Assume Role credentials (keyed by profile name)
static ASSUME_ROLE_CACHE: OnceLock<std::sync::Mutex<HashMap<String, CachedImdsCredentials>>> =
    OnceLock::new();

/// Global cache for ECS container credentials
static ECS_CACHE: OnceLock<std::sync::Mutex<Option<CachedImdsCredentials>>> = OnceLock::new();

/// ECS container credentials endpoint base
const ECS_CREDENTIALS_ENDPOINT: &str = "http://169.254.170.2";

/// IMDSv2 metadata endpoint
const IMDS_ENDPOINT: &str = "http://169.254.169.254";
/// IMDSv2 token TTL in seconds (6 hours)
const IMDS_TOKEN_TTL: u64 = 21600;
/// Timeout for IMDS requests (2 seconds - fast fail if not on EC2)
const IMDS_TIMEOUT: Duration = Duration::from_secs(2);
/// Refresh credentials 5 minutes before expiration
const CREDENTIAL_REFRESH_BUFFER: Duration = Duration::from_secs(300);

/// Load credentials for a given profile
pub fn load_credentials(profile: &str) -> Result<Credentials> {
    load_credentials_inner(profile).map_err(|e| match e {
        CredentialsError::SsoLoginRequired {
            profile,
            sso_session,
        } => {
            anyhow!(
                "SSO login required for profile '{}' (session: {})",
                profile,
                sso_session
            )
        }
        CredentialsError::Other(e) => e,
    })
}

/// Load credentials with detailed error for SSO
pub fn load_credentials_with_sso_check(profile: &str) -> Result<Credentials, CredentialsError> {
    load_credentials_inner(profile)
}

/// Internal credential loading with specific SSO error
fn load_credentials_inner(profile: &str) -> Result<Credentials, CredentialsError> {
    // 1. Try environment variables first (if default profile or explicitly set)
    if profile == "default" {
        if let Ok(creds) = load_from_env() {
            debug!("Loaded credentials from environment variables");
            return Ok(creds);
        }
    }

    // 2. Check if SSO is configured for this profile - if so, prioritize SSO
    //    This ensures we don't use stale static credentials when SSO is the intended auth method
    if let Some(sso_config) = super::sso::get_sso_config(profile) {
        debug!(
            "SSO is configured for profile '{}', trying SSO first",
            profile
        );
        match load_from_sso(profile) {
            Ok(creds) => {
                debug!("Loaded credentials from AWS SSO for profile '{}'", profile);
                return Ok(creds);
            }
            Err(e) => {
                debug!(
                    "SSO configured for profile '{}' but token unavailable: {}",
                    profile, e
                );
                return Err(CredentialsError::SsoLoginRequired {
                    profile: profile.to_string(),
                    sso_session: sso_config.sso_session,
                });
            }
        }
    }

    // 3. Check if role_arn is configured for this profile (role assumption)
    if let Some(assume_role_config) = get_assume_role_config(profile) {
        debug!(
            "Role assumption configured for profile '{}', role_arn: {}",
            profile, assume_role_config.role_arn
        );

        // 3a. First try to read from AWS CLI cache (credentials from `aws` CLI commands)
        if let Ok(creds) = load_from_cli_cache(profile, &assume_role_config.role_arn) {
            debug!(
                "Loaded credentials from AWS CLI cache for profile '{}'",
                profile
            );
            return Ok(creds);
        }

        // 3b. Fall back to performing role assumption ourselves
        match load_from_assume_role(profile, &assume_role_config) {
            Ok(creds) => {
                debug!(
                    "Loaded credentials via role assumption for profile '{}'",
                    profile
                );
                return Ok(creds);
            }
            Err(e) => {
                debug!("Role assumption failed for profile '{}': {}", profile, e);
                return Err(CredentialsError::Other(e));
            }
        }
    }

    // 5. Try AWS credentials file
    if let Ok(creds) = load_from_credentials_file(profile) {
        debug!(
            "Loaded credentials from credentials file for profile '{}'",
            profile
        );
        return Ok(creds);
    }

    // 6. Try config file with direct credentials
    if let Ok(creds) = load_from_config_file(profile) {
        debug!(
            "Loaded credentials from config file for profile '{}'",
            profile
        );
        return Ok(creds);
    }

    // 7. Try IMDSv2 (EC2 instance metadata) - only for default profile
    if profile == "default" {
        match load_from_imds() {
            Ok(creds) => {
                debug!("Loaded credentials from EC2 instance metadata (IMDSv2)");
                return Ok(creds);
            }
            Err(e) => {
                debug!("IMDSv2 credential loading failed: {}", e);
            }
        }
    }

    Err(CredentialsError::Other(anyhow!(
        "No credentials found for profile '{}'. Run 'aws configure' or 'aws sso login --profile {}' or set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY",
        profile,
        profile
    )))
}

/// Load credentials from environment variables
fn load_from_env() -> Result<Credentials> {
    let access_key_id =
        env::var("AWS_ACCESS_KEY_ID").map_err(|_| anyhow!("AWS_ACCESS_KEY_ID not set"))?;
    let secret_access_key =
        env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| anyhow!("AWS_SECRET_ACCESS_KEY not set"))?;
    let session_token = env::var("AWS_SESSION_TOKEN").ok();

    Ok(Credentials {
        access_key_id,
        secret_access_key,
        session_token,
    })
}

/// Get AWS config directory
pub fn aws_config_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("AWS_CONFIG_FILE") {
        if let Some(parent) = PathBuf::from(path).parent() {
            return Ok(parent.to_path_buf());
        }
    }

    dirs::home_dir()
        .map(|h| h.join(".aws"))
        .ok_or_else(|| anyhow!("Could not find home directory"))
}

/// Get the AWS config file path, respecting AWS_CONFIG_FILE environment variable
pub fn get_aws_config_file_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("AWS_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }

    dirs::home_dir()
        .map(|h| h.join(".aws").join("config"))
        .ok_or_else(|| anyhow!("Could not find home directory"))
}

/// Parse an INI-style file into sections
/// Returns (profiles, sso_sessions) where sso_sessions contains [sso-session X] sections
fn parse_ini_file(content: &str) -> HashMap<String, HashMap<String, String>> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_string();
            // Handle "profile name" format in config file
            if current_section.starts_with("profile ") {
                current_section = current_section["profile ".len()..].to_string();
            }
            // Keep sso-session sections with their prefix for identification
            sections.entry(current_section.clone()).or_default();
            continue;
        }

        // Key-value pair
        if let Some((key, value)) = line.split_once('=') {
            if !current_section.is_empty() {
                sections
                    .entry(current_section.clone())
                    .or_default()
                    .insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    sections
}

/// Load credentials from ~/.aws/credentials or AWS_SHARED_CREDENTIALS_FILE
fn load_from_credentials_file(profile: &str) -> Result<Credentials> {
    // Check AWS_SHARED_CREDENTIALS_FILE env var first (AWS SDK standard)
    let creds_path = if let Ok(path) = env::var("AWS_SHARED_CREDENTIALS_FILE") {
        PathBuf::from(path)
    } else {
        aws_config_dir()?.join("credentials")
    };
    let content =
        fs::read_to_string(&creds_path).map_err(|_| anyhow!("Could not read {:?}", creds_path))?;

    let sections = parse_ini_file(&content);

    let section = sections
        .get(profile)
        .ok_or_else(|| anyhow!("Profile '{}' not found in credentials file", profile))?;

    if let Some(command) = section.get("credential_process") {
        debug!("Found credential_process for profile '{}'", profile);
        return load_from_process(profile, command);
    }

    let access_key_id = section
        .get("aws_access_key_id")
        .ok_or_else(|| anyhow!("aws_access_key_id not found for profile '{}'", profile))?
        .clone();

    let secret_access_key = section
        .get("aws_secret_access_key")
        .ok_or_else(|| anyhow!("aws_secret_access_key not found for profile '{}'", profile))?
        .clone();

    let session_token = section.get("aws_session_token").cloned();

    Ok(Credentials {
        access_key_id,
        secret_access_key,
        session_token,
    })
}

/// Load credentials from ~/.aws/config (for direct credentials only)
fn load_from_config_file(profile: &str) -> Result<Credentials> {
    let config_path = get_aws_config_file_path()?;
    let content = fs::read_to_string(&config_path)
        .map_err(|_| anyhow!("Could not read {:?}", config_path))?;

    let sections = parse_ini_file(&content);

    let section = sections
        .get(profile)
        .ok_or_else(|| anyhow!("Profile '{}' not found in config file", profile))?;

    if let Some(command) = section.get("credential_process") {
        debug!("Found credential_process for profile '{}'", profile);
        return load_from_process(profile, command);
    }

    // Check for direct credentials in config (less common but valid)
    if let (Some(access_key), Some(secret_key)) = (
        section.get("aws_access_key_id"),
        section.get("aws_secret_access_key"),
    ) {
        return Ok(Credentials {
            access_key_id: access_key.clone(),
            secret_access_key: secret_key.clone(),
            session_token: section.get("aws_session_token").cloned(),
        });
    }

    Err(anyhow!(
        "No direct credentials found in config for profile '{}'",
        profile
    ))
}

// =============================================================================
// AWS SSO (IAM Identity Center) Support
// =============================================================================

/// Load credentials from AWS SSO (IAM Identity Center)
/// This only works with cached tokens - for interactive login, use the sso module directly
fn load_from_sso(profile: &str) -> Result<Credentials> {
    use super::sso;

    // Check credential cache first (keyed by profile)
    let cache = SSO_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(profile) {
            if cached.expiration > Instant::now() + CREDENTIAL_REFRESH_BUFFER {
                trace!("Using cached SSO credentials for profile '{}'", profile);
                return Ok(cached.credentials.clone());
            }
        }
    }

    // Get SSO config for this profile
    let sso_config = sso::get_sso_config(profile)
        .ok_or_else(|| anyhow!("Profile '{}' does not have SSO configured", profile))?;

    // Try to read cached SSO token
    let access_token = sso::read_cached_token(&sso_config).ok_or_else(|| {
        anyhow!(
            "SSO token not found or expired for profile '{}'. Interactive login required.",
            profile
        )
    })?;

    // Exchange token for credentials
    let credentials = sso::get_role_credentials(&sso_config, &access_token)?;

    // Cache the credentials (keyed by profile)
    let expiration = Instant::now() + Duration::from_secs(3600); // Default 1 hour
    let cache = SSO_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            profile.to_string(),
            CachedImdsCredentials {
                credentials: credentials.clone(),
                expiration,
            },
        );
        debug!("Cached SSO credentials for profile '{}'", profile);
    }

    Ok(credentials)
}

// =============================================================================
// AWS CLI Cache Support
// =============================================================================

/// Load credentials from AWS CLI cache directory (~/.aws/cli/cache/)
/// The AWS CLI caches assumed role credentials when using profiles with role_arn
fn load_from_cli_cache(profile: &str, role_arn: &str) -> Result<Credentials> {
    let cache_dir = aws_config_dir()?.join("cli").join("cache");

    if !cache_dir.exists() {
        return Err(anyhow!("AWS CLI cache directory not found"));
    }

    trace!(
        "Searching AWS CLI cache for role_arn: {} (profile: {})",
        role_arn,
        profile
    );

    // Search through all cache files to find one matching this role_arn
    let entries = fs::read_dir(&cache_dir)
        .map_err(|e| anyhow!("Failed to read CLI cache directory: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process .json files
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        // Try to read and parse the cache file
        if let Some(creds) = try_read_cli_cache_file(&path, role_arn) {
            debug!("Found valid credentials in CLI cache: {:?}", path);
            return Ok(creds);
        }
    }

    Err(anyhow!(
        "No valid cached credentials found for profile '{}'",
        profile
    ))
}

/// Try to read credentials from a CLI cache file if it matches the role_arn
fn try_read_cli_cache_file(path: &std::path::Path, role_arn: &str) -> Option<Credentials> {
    let content = fs::read_to_string(path).ok()?;
    let cache_data: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check if this cache file matches our role_arn
    // AssumedRoleUser ARN format: arn:aws:sts::account-id:assumed-role/role-name/session-name
    // role_arn format: arn:aws:iam::account-id:role/role-name
    let assumed_role_arn = cache_data
        .get("AssumedRoleUser")
        .and_then(|u| u.get("Arn"))
        .and_then(|a| a.as_str())?;

    // Extract role name and account from both ARNs and compare
    let cache_parts: Vec<&str> = assumed_role_arn.split(':').collect();
    let config_parts: Vec<&str> = role_arn.split(':').collect();

    // Compare account IDs (index 4)
    if cache_parts.get(4) != config_parts.get(4) {
        return None;
    }

    // Extract role names
    // assumed-role ARN: "assumed-role/role-name/session" -> role-name is second part
    // role ARN: "role/role-name" -> role-name is second part
    let cache_role_name = assumed_role_arn.split('/').nth(1)?;
    let config_role_name = role_arn.split('/').next_back()?;

    if cache_role_name != config_role_name {
        return None;
    }

    // Extract credentials
    let creds = cache_data.get("Credentials")?;

    let access_key_id = creds.get("AccessKeyId").and_then(|v| v.as_str())?;
    let secret_access_key = creds.get("SecretAccessKey").and_then(|v| v.as_str())?;
    let session_token = creds.get("SessionToken").and_then(|v| v.as_str());

    // Check expiration
    if let Some(expiration_str) = creds.get("Expiration").and_then(|v| v.as_str()) {
        if let Ok(expiration) = chrono::DateTime::parse_from_rfc3339(expiration_str) {
            if expiration <= chrono::Utc::now() {
                trace!("CLI cache credentials expired: {:?}", path);
                return None;
            }
            trace!(
                "CLI cache credentials valid until: {} (file: {:?})",
                expiration,
                path
            );
        }
    }

    Some(Credentials {
        access_key_id: access_key_id.to_string(),
        secret_access_key: secret_access_key.to_string(),
        session_token: session_token.map(|s| s.to_string()),
    })
}

// =============================================================================
// Process Credentials Support
// =============================================================================

/// Load credentials from external process
fn load_from_process(profile: &str, command: &str) -> Result<Credentials> {
    // Check cache first
    let cache = PROCESS_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(profile) {
            // If the credentials are still valid, use them
            if cached.expiration > Instant::now() + CREDENTIAL_REFRESH_BUFFER {
                trace!("Using cached process credentials for profile '{}'", profile);
                return Ok(cached.credentials.clone());
            }
        }
    }

    // Execute command
    let (credentials, expiration) = execute_credential_process(command)?;

    // Determine cache expiration
    // If expiration is provided, use it (temporary credentials).
    // If not, treat as long-term credentials and cache for a very long time
    // to avoid re-running the process unnecessarily.
    let cache_expiration = expiration.unwrap_or_else(|| {
        Instant::now() + Duration::from_secs(365 * 24 * 60 * 60) // 1 year
    });

    // Cache the credentials
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            profile.to_string(),
            CachedImdsCredentials {
                credentials: credentials.clone(),
                expiration: cache_expiration,
            },
        );
        if expiration.is_some() {
            debug!(
                "Cached temporary process credentials for profile '{}'",
                profile
            );
        } else {
            debug!(
                "Cached long-term process credentials for profile '{}'",
                profile
            );
        }
    }

    Ok(credentials)
}

fn execute_credential_process(command: &str) -> Result<(Credentials, Option<Instant>)> {
    debug!("Executing credential_process: {}", command);

    #[cfg(not(windows))]
    let shell_cmd = Command::new("sh").arg("-c").arg(command).output();

    #[cfg(windows)]
    let shell_cmd = Command::new("cmd").arg("/C").arg(command).output();

    let output = shell_cmd.map_err(|e| anyhow!("Failed to execute credential_process: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "credential_process failed with status {}: {}",
            output.status,
            stderr
        ));
    }

    let output_str = String::from_utf8(output.stdout)
        .map_err(|e| anyhow!("Invalid UTF-8 output from credential_process: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&output_str)
        .map_err(|e| anyhow!("Failed to parse credential_process output: {}", e))?;

    // Check version (should be 1)
    if let Some(version) = json.get("Version").and_then(|v| v.as_i64()) {
        if version != 1 {
            return Err(anyhow!(
                "Unsupported credential_process version: {}",
                version
            ));
        }
    }

    let access_key_id = json
        .get("AccessKeyId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("AccessKeyId missing in credential_process output"))?
        .to_string();

    let secret_access_key = json
        .get("SecretAccessKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("SecretAccessKey missing in credential_process output"))?
        .to_string();

    let session_token = json
        .get("SessionToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let expiration = json
        .get("Expiration")
        .and_then(|v| v.as_str())
        .and_then(parse_expiration);

    Ok((
        Credentials {
            access_key_id,
            secret_access_key,
            session_token,
        },
        expiration,
    ))
}

/// Get the default region for a profile
#[allow(dead_code)]
pub fn get_profile_region(profile: &str) -> Option<String> {
    // 1. Check environment variable
    if let Ok(region) = env::var("AWS_REGION") {
        return Some(region);
    }
    if let Ok(region) = env::var("AWS_DEFAULT_REGION") {
        return Some(region);
    }

    // 2. Check config file
    if let Ok(config_dir) = aws_config_dir() {
        let config_path = config_dir.join("config");
        if let Ok(content) = fs::read_to_string(&config_path) {
            let sections = parse_ini_file(&content);
            if let Some(section) = sections.get(profile) {
                if let Some(region) = section.get("region") {
                    return Some(region.clone());
                }
            }
        }
    }

    None
}

/// List available AWS profiles
#[allow(dead_code)]
pub fn list_profiles() -> Vec<String> {
    let mut profiles = Vec::new();

    if let Ok(config_dir) = aws_config_dir() {
        // Read from credentials file
        if let Ok(content) = fs::read_to_string(config_dir.join("credentials")) {
            let sections = parse_ini_file(&content);
            profiles.extend(sections.keys().cloned());
        }

        // Read from config file
        if let Ok(content) = fs::read_to_string(config_dir.join("config")) {
            let sections = parse_ini_file(&content);
            for key in sections.keys() {
                if !profiles.contains(key) {
                    profiles.push(key.clone());
                }
            }
        }
    }

    profiles.sort();
    profiles
}

// =============================================================================
// IAM Role Assumption Support (role_arn + source_profile)
// =============================================================================

/// Configuration for assuming an IAM role
#[derive(Debug, Clone)]
pub struct AssumeRoleConfig {
    /// The ARN of the role to assume
    pub role_arn: String,
    /// The source profile to use for credentials (mutually exclusive with credential_source)
    pub source_profile: Option<String>,
    /// The credential source type (mutually exclusive with source_profile)
    /// Valid values: "Environment", "Ec2InstanceMetadata", "EcsContainer"
    pub credential_source: Option<CredentialSource>,
    /// Optional external ID for cross-account access
    pub external_id: Option<String>,
    /// Optional role session name (defaults to "taws-session")
    pub role_session_name: Option<String>,
    /// Optional duration in seconds (defaults to 3600)
    pub duration_seconds: Option<u32>,
    /// Region for STS endpoint (from source profile or default)
    pub region: Option<String>,
}

/// Supported credential sources for role assumption
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialSource {
    /// Load credentials from environment variables
    Environment,
    /// Load credentials from EC2 instance metadata (IMDSv2)
    Ec2InstanceMetadata,
    /// Load credentials from ECS container credentials endpoint
    EcsContainer,
}

/// Check if role assumption is configured for a profile
fn get_assume_role_config(profile: &str) -> Option<AssumeRoleConfig> {
    // Respect AWS_CONFIG_FILE environment variable
    let config_path = if let Ok(path) = env::var("AWS_CONFIG_FILE") {
        PathBuf::from(path)
    } else {
        aws_config_dir().ok()?.join("config")
    };
    let content = fs::read_to_string(&config_path).ok()?;
    let sections = parse_ini_file(&content);

    let section = sections.get(profile)?;

    // Must have role_arn to be a role assumption profile
    let role_arn = section.get("role_arn")?.clone();

    // Get source_profile and credential_source
    let source_profile = section.get("source_profile").cloned();
    let credential_source = section
        .get("credential_source")
        .and_then(|s| parse_credential_source(s));

    // Must have exactly one of source_profile or credential_source
    match (&source_profile, &credential_source) {
        (Some(_), Some(_)) => {
            debug!(
                "Profile '{}' has both source_profile and credential_source - invalid configuration",
                profile
            );
            return None;
        }
        (None, None) => {
            debug!(
                "Profile '{}' has role_arn but neither source_profile nor credential_source",
                profile
            );
            return None;
        }
        _ => {}
    }

    Some(AssumeRoleConfig {
        role_arn,
        source_profile,
        credential_source,
        external_id: section.get("external_id").cloned(),
        role_session_name: section.get("role_session_name").cloned(),
        duration_seconds: section.get("duration_seconds").and_then(|s| s.parse().ok()),
        region: section.get("region").cloned(),
    })
}

/// Parse credential_source string value
fn parse_credential_source(value: &str) -> Option<CredentialSource> {
    match value {
        "Environment" => Some(CredentialSource::Environment),
        "Ec2InstanceMetadata" => Some(CredentialSource::Ec2InstanceMetadata),
        "EcsContainer" => Some(CredentialSource::EcsContainer),
        _ => {
            debug!("Unknown credential_source value: {}", value);
            None
        }
    }
}

/// Load credentials by assuming a role using source profile or credential_source
fn load_from_assume_role(profile: &str, config: &AssumeRoleConfig) -> Result<Credentials> {
    // Check cache first
    let cache = ASSUME_ROLE_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(profile) {
            if cached.expiration > Instant::now() + CREDENTIAL_REFRESH_BUFFER {
                trace!(
                    "Using cached assume role credentials for profile '{}'",
                    profile
                );
                return Ok(cached.credentials.clone());
            }
        }
    }

    // Load source credentials based on configuration
    let source_creds = if let Some(ref source_profile) = config.source_profile {
        // Recursively load credentials from source profile
        // This handles chained role assumption (source_profile can also use role_arn)
        debug!(
            "Loading source credentials from profile '{}'",
            source_profile
        );
        load_credentials(source_profile).map_err(|e| {
            anyhow!(
                "Failed to load source credentials from profile '{}': {}",
                source_profile,
                e
            )
        })?
    } else if let Some(ref credential_source) = config.credential_source {
        // Load credentials from credential_source
        debug!(
            "Loading source credentials from credential_source: {:?}",
            credential_source
        );
        load_from_credential_source(credential_source)?
    } else {
        return Err(anyhow!(
            "Profile '{}' has role_arn but no source_profile or credential_source",
            profile
        ));
    };

    // Determine region for STS call
    let region = config
        .region
        .clone()
        .or_else(|| {
            config
                .source_profile
                .as_ref()
                .and_then(|p| get_profile_region(p))
        })
        .unwrap_or_else(|| "us-east-1".to_string());

    // Call STS AssumeRole
    let (credentials, expiration) = call_sts_assume_role(config, &source_creds, &region)?;

    // Cache the credentials
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            profile.to_string(),
            CachedImdsCredentials {
                credentials: credentials.clone(),
                expiration,
            },
        );
        debug!(
            "Cached assume role credentials for profile '{}', expires in {:?}",
            profile,
            expiration - Instant::now()
        );
    }

    Ok(credentials)
}

/// Load credentials from a credential_source
fn load_from_credential_source(source: &CredentialSource) -> Result<Credentials> {
    match source {
        CredentialSource::Environment => {
            debug!("Loading credentials from Environment");
            load_from_env()
        }
        CredentialSource::Ec2InstanceMetadata => {
            debug!("Loading credentials from Ec2InstanceMetadata (IMDSv2)");
            load_from_imds()
        }
        CredentialSource::EcsContainer => {
            debug!("Loading credentials from EcsContainer");
            load_from_ecs_container()
        }
    }
}

/// Call STS AssumeRole API using signed HTTP request
fn call_sts_assume_role(
    config: &AssumeRoleConfig,
    source_creds: &Credentials,
    region: &str,
) -> Result<(Credentials, Instant)> {
    use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
    use aws_sigv4::sign::v4::SigningParams;
    use aws_smithy_runtime_api::client::identity::Identity;
    use std::time::SystemTime;

    let role_session_name = config
        .role_session_name
        .clone()
        .unwrap_or_else(|| "taws-session".to_string());
    let duration_seconds = config.duration_seconds.unwrap_or(3600);

    // Build STS endpoint - respect AWS_ENDPOINT_URL or TAWS_STS_ENDPOINT for LocalStack/testing
    let sts_endpoint = env::var("TAWS_STS_ENDPOINT")
        .or_else(|_| env::var("AWS_ENDPOINT_URL"))
        .unwrap_or_else(|_| format!("https://sts.{}.amazonaws.com", region));

    // Build query parameters
    let mut params = vec![
        ("Action", "AssumeRole"),
        ("Version", "2011-06-15"),
        ("RoleArn", &config.role_arn),
        ("RoleSessionName", &role_session_name),
    ];

    let duration_str = duration_seconds.to_string();
    params.push(("DurationSeconds", &duration_str));

    if let Some(ref external_id) = config.external_id {
        params.push(("ExternalId", external_id));
    }

    let query_string: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    // Build full URL - handle endpoints with or without trailing slash
    let base = sts_endpoint.trim_end_matches('/');
    let url = format!("{}/?{}", base, query_string);

    debug!("Calling STS AssumeRole: {}", config.role_arn);
    debug!("STS endpoint: {}", sts_endpoint);
    trace!("STS URL: {}", url);

    // Parse URL for signing
    let parsed_url = url::Url::parse(&url)?;
    let host = parsed_url
        .host_str()
        .ok_or_else(|| anyhow!("Invalid STS URL"))?;
    let path_and_query = if let Some(query) = parsed_url.query() {
        format!("{}?{}", parsed_url.path(), query)
    } else {
        parsed_url.path().to_string()
    };

    // Build headers for signing
    let headers = [("host".to_string(), host.to_string())];

    // Create identity for signing
    let creds = aws_credential_types::Credentials::new(
        &source_creds.access_key_id,
        &source_creds.secret_access_key,
        source_creds.session_token.clone(),
        None,
        "taws",
    );
    let identity: Identity = creds.into();

    // Create signing params
    let signing_params = SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("sts")
        .time(SystemTime::now())
        .settings(SigningSettings::default())
        .build()?
        .into();

    // Create signable request
    let signable_request = SignableRequest::new(
        "POST",
        &path_and_query,
        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        SignableBody::Bytes(&[]),
    )?;

    // Sign the request
    let (signing_instructions, _signature) = sign(signable_request, &signing_params)?.into_parts();

    // Build and send the request
    let client = super::tls::create_blocking_client_with_timeout(Duration::from_secs(30))?;

    let mut request = client.post(&url);

    // Apply signing headers
    for (name, value) in signing_instructions.headers() {
        request = request.header(name.to_string(), value.to_string());
    }

    let response = request.send()?;
    let status = response.status();
    let text = response.text()?;

    if !status.is_success() {
        // Parse error message from XML response
        let error_msg = parse_sts_error(&text).unwrap_or_else(|| text.clone());
        return Err(anyhow!("STS AssumeRole failed ({}): {}", status, error_msg));
    }

    // Parse the XML response
    parse_assume_role_response(&text)
}

/// Parse STS error response
fn parse_sts_error(xml: &str) -> Option<String> {
    // Simple XML parsing for error message
    // Format: <Error><Code>...</Code><Message>...</Message></Error>
    let code_start = xml.find("<Code>")? + 6;
    let code_end = xml.find("</Code>")?;
    let code = &xml[code_start..code_end];

    let msg_start = xml.find("<Message>")? + 9;
    let msg_end = xml.find("</Message>")?;
    let message = &xml[msg_start..msg_end];

    Some(format!("{}: {}", code, message))
}

/// Parse AssumeRole XML response
fn parse_assume_role_response(xml: &str) -> Result<(Credentials, Instant)> {
    // Parse XML response for credentials
    // Format: <AssumeRoleResponse><AssumeRoleResult><Credentials>...</Credentials></AssumeRoleResult></AssumeRoleResponse>

    let extract_value = |tag: &str| -> Option<String> {
        let start_tag = format!("<{}>", tag);
        let end_tag = format!("</{}>", tag);
        let start = xml.find(&start_tag)? + start_tag.len();
        let end = xml.find(&end_tag)?;
        if start < end {
            Some(xml[start..end].to_string())
        } else {
            None
        }
    };

    let access_key_id = extract_value("AccessKeyId")
        .ok_or_else(|| anyhow!("AccessKeyId not found in AssumeRole response"))?;

    let secret_access_key = extract_value("SecretAccessKey")
        .ok_or_else(|| anyhow!("SecretAccessKey not found in AssumeRole response"))?;

    let session_token = extract_value("SessionToken")
        .ok_or_else(|| anyhow!("SessionToken not found in AssumeRole response"))?;

    let expiration_str = extract_value("Expiration")
        .ok_or_else(|| anyhow!("Expiration not found in AssumeRole response"))?;

    let expiration = parse_expiration(&expiration_str)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));

    Ok((
        Credentials {
            access_key_id,
            secret_access_key,
            session_token: Some(session_token),
        },
        expiration,
    ))
}

// =============================================================================
// IMDSv2 (EC2 Instance Metadata Service) Support
// =============================================================================

/// Load credentials from EC2 Instance Metadata Service (IMDSv2)
///
/// This function:
/// 1. Checks if we have valid cached credentials
/// 2. If not, fetches a session token from IMDSv2
/// 3. Uses the token to get the IAM role name
/// 4. Fetches temporary credentials for that role
/// 5. Caches the credentials until near expiration
fn load_from_imds() -> Result<Credentials> {
    // Check cache first
    let cache = IMDS_CACHE.get_or_init(|| std::sync::Mutex::new(None));

    if let Ok(guard) = cache.lock() {
        if let Some(ref cached) = *guard {
            // Return cached credentials if not expired (with buffer)
            if cached.expiration > Instant::now() + CREDENTIAL_REFRESH_BUFFER {
                trace!("Using cached IMDS credentials");
                return Ok(cached.credentials.clone());
            }
        }
    }

    // Fetch fresh credentials
    let creds = fetch_imds_credentials()?;

    Ok(creds)
}

/// Fetch credentials from IMDSv2 endpoint
fn fetch_imds_credentials() -> Result<Credentials> {
    // Use a blocking HTTP client with short timeout
    let client = reqwest::blocking::Client::builder()
        .timeout(IMDS_TIMEOUT)
        .connect_timeout(IMDS_TIMEOUT)
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

    // Step 1: Get IMDSv2 session token
    trace!("Fetching IMDSv2 session token");
    let token_url = format!("{}/latest/api/token", IMDS_ENDPOINT);
    let token_response = client
        .put(&token_url)
        .header(
            "X-aws-ec2-metadata-token-ttl-seconds",
            IMDS_TOKEN_TTL.to_string(),
        )
        .send()
        .map_err(|e| anyhow!("Failed to get IMDS token (not running on EC2?): {}", e))?;

    if !token_response.status().is_success() {
        return Err(anyhow!(
            "IMDS token request failed with status: {}",
            token_response.status()
        ));
    }

    let token = token_response
        .text()
        .map_err(|e| anyhow!("Failed to read IMDS token: {}", e))?;

    // Step 2: Get IAM role name
    trace!("Fetching IAM role name from IMDS");
    let role_url = format!(
        "{}/latest/meta-data/iam/security-credentials/",
        IMDS_ENDPOINT
    );
    let role_response = client
        .get(&role_url)
        .header("X-aws-ec2-metadata-token", &token)
        .send()
        .map_err(|e| anyhow!("Failed to get IAM role: {}", e))?;

    if !role_response.status().is_success() {
        return Err(anyhow!(
            "No IAM role attached to this EC2 instance (status: {})",
            role_response.status()
        ));
    }

    let role_text = role_response
        .text()
        .map_err(|e| anyhow!("Failed to read IAM role name: {}", e))?;

    // Take the first role if multiple are returned (newline-separated)
    let role_name = role_text
        .lines()
        .next()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("No IAM role attached to this EC2 instance"))?
        .to_string();

    debug!("Found IAM role: {}", role_name);

    // Step 3: Get credentials for the role
    trace!("Fetching credentials for IAM role: {}", role_name);
    let creds_url = format!(
        "{}/latest/meta-data/iam/security-credentials/{}",
        IMDS_ENDPOINT, role_name
    );
    let creds_response = client
        .get(&creds_url)
        .header("X-aws-ec2-metadata-token", &token)
        .send()
        .map_err(|e| anyhow!("Failed to get credentials: {}", e))?;

    if !creds_response.status().is_success() {
        return Err(anyhow!(
            "Failed to get credentials for role '{}' (status: {})",
            role_name,
            creds_response.status()
        ));
    }

    let creds_json: serde_json::Value = creds_response
        .json()
        .map_err(|e| anyhow!("Failed to parse credentials JSON: {}", e))?;

    // Parse the credentials
    let access_key_id = creds_json
        .get("AccessKeyId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("AccessKeyId not found in IMDS response"))?
        .to_string();

    let secret_access_key = creds_json
        .get("SecretAccessKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("SecretAccessKey not found in IMDS response"))?
        .to_string();

    let session_token = creds_json
        .get("Token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Parse expiration time
    let expiration = if let Some(exp_str) = creds_json.get("Expiration").and_then(|v| v.as_str()) {
        // Parse ISO 8601 format: "2024-01-15T12:00:00Z"
        parse_expiration(exp_str).unwrap_or_else(|| {
            // Default to 1 hour if parsing fails
            Instant::now() + Duration::from_secs(3600)
        })
    } else {
        // Default to 1 hour if no expiration provided
        Instant::now() + Duration::from_secs(3600)
    };

    let credentials = Credentials {
        access_key_id,
        secret_access_key,
        session_token,
    };

    // Cache the credentials
    let cache = IMDS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedImdsCredentials {
            credentials: credentials.clone(),
            expiration,
        });
        debug!(
            "Cached IMDS credentials, expires in {:?}",
            expiration - Instant::now()
        );
    }

    Ok(credentials)
}

/// Parse ISO 8601 expiration time to Instant
fn parse_expiration(exp_str: &str) -> Option<Instant> {
    // Parse "2024-01-15T12:00:00Z" format
    use chrono::{DateTime, Utc};

    let expiration_time: DateTime<Utc> = exp_str.parse().ok()?;
    let now = Utc::now();

    if expiration_time <= now {
        return None;
    }

    let duration_until_expiration = (expiration_time - now).to_std().ok()?;
    Some(Instant::now() + duration_until_expiration)
}

/// Check if IMDS is available (useful for detecting EC2 environment)
#[allow(dead_code)]
pub fn is_imds_available() -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(IMDS_TIMEOUT)
        .connect_timeout(IMDS_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let token_url = format!("{}/latest/api/token", IMDS_ENDPOINT);
    client
        .put(&token_url)
        .header("X-aws-ec2-metadata-token-ttl-seconds", "21600")
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

// =============================================================================
// ECS Container Credentials Support
// =============================================================================

/// Load credentials from ECS container credentials endpoint
///
/// This function:
/// 1. Checks for AWS_CONTAINER_CREDENTIALS_RELATIVE_URI or AWS_CONTAINER_CREDENTIALS_FULL_URI
/// 2. Fetches credentials from the ECS metadata endpoint
/// 3. Caches the credentials until near expiration
fn load_from_ecs_container() -> Result<Credentials> {
    // Check cache first
    let cache = ECS_CACHE.get_or_init(|| std::sync::Mutex::new(None));

    if let Ok(guard) = cache.lock() {
        if let Some(ref cached) = *guard {
            if cached.expiration > Instant::now() + CREDENTIAL_REFRESH_BUFFER {
                trace!("Using cached ECS container credentials");
                return Ok(cached.credentials.clone());
            }
        }
    }

    // Fetch fresh credentials
    let (credentials, expiration) = fetch_ecs_container_credentials()?;

    // Cache the credentials
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedImdsCredentials {
            credentials: credentials.clone(),
            expiration,
        });
        debug!(
            "Cached ECS container credentials, expires in {:?}",
            expiration - Instant::now()
        );
    }

    Ok(credentials)
}

/// Fetch credentials from ECS container credentials endpoint
fn fetch_ecs_container_credentials() -> Result<(Credentials, Instant)> {
    // Determine the credentials URL
    // Priority: AWS_CONTAINER_CREDENTIALS_FULL_URI > AWS_CONTAINER_CREDENTIALS_RELATIVE_URI
    let (url, auth_token) = if let Ok(full_uri) = env::var("AWS_CONTAINER_CREDENTIALS_FULL_URI") {
        // Full URI mode - may require authorization token
        let token = env::var("AWS_CONTAINER_AUTHORIZATION_TOKEN").ok();
        debug!("Using ECS full URI: {}", full_uri);
        (full_uri, token)
    } else if let Ok(relative_uri) = env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
        // Relative URI mode - use the standard ECS endpoint
        let url = format!("{}{}", ECS_CREDENTIALS_ENDPOINT, relative_uri);
        debug!("Using ECS relative URI: {}", url);
        (url, None)
    } else {
        return Err(anyhow!(
            "ECS container credentials not available: neither AWS_CONTAINER_CREDENTIALS_FULL_URI \
             nor AWS_CONTAINER_CREDENTIALS_RELATIVE_URI is set"
        ));
    };

    // Create HTTP client with timeout
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

    // Build request
    let mut request = client.get(&url);

    // Add authorization header if token is present
    if let Some(ref token) = auth_token {
        request = request.header("Authorization", token);
    }

    // Send request
    trace!("Fetching ECS container credentials from: {}", url);
    let response = request
        .send()
        .map_err(|e| anyhow!("Failed to fetch ECS container credentials: {}", e))?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "ECS container credentials request failed with status: {}",
            response.status()
        ));
    }

    // Parse JSON response
    let creds_json: serde_json::Value = response
        .json()
        .map_err(|e| anyhow!("Failed to parse ECS credentials JSON: {}", e))?;

    // Extract credentials
    let access_key_id = creds_json
        .get("AccessKeyId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("AccessKeyId not found in ECS credentials response"))?
        .to_string();

    let secret_access_key = creds_json
        .get("SecretAccessKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("SecretAccessKey not found in ECS credentials response"))?
        .to_string();

    let session_token = creds_json
        .get("Token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Parse expiration time
    let expiration = if let Some(exp_str) = creds_json.get("Expiration").and_then(|v| v.as_str()) {
        parse_expiration(exp_str).unwrap_or_else(|| Instant::now() + Duration::from_secs(3600))
    } else {
        Instant::now() + Duration::from_secs(3600)
    };

    debug!(
        "Fetched ECS container credentials, expires in {:?}",
        expiration - Instant::now()
    );

    Ok((
        Credentials {
            access_key_id,
            secret_access_key,
            session_token,
        },
        expiration,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sso_cache_is_profile_aware() {
        // This test verifies that SSO credentials are cached per-profile,
        // not globally. This is a regression test for the bug where switching
        // profiles would return cached credentials from the previous profile.

        let cache = SSO_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

        // Create credentials for two different profiles
        let creds_profile_a = Credentials {
            access_key_id: "AKIAPROFILE_A_KEY".to_string(),
            secret_access_key: "secret_a".to_string(),
            session_token: Some("token_a".to_string()),
        };
        let creds_profile_b = Credentials {
            access_key_id: "AKIAPROFILE_B_KEY".to_string(),
            secret_access_key: "secret_b".to_string(),
            session_token: Some("token_b".to_string()),
        };

        let expiration = Instant::now() + Duration::from_secs(3600);

        // Cache credentials for both profiles
        {
            let mut guard = cache.lock().unwrap();
            guard.insert(
                "profile-a".to_string(),
                CachedImdsCredentials {
                    credentials: creds_profile_a.clone(),
                    expiration,
                },
            );
            guard.insert(
                "profile-b".to_string(),
                CachedImdsCredentials {
                    credentials: creds_profile_b.clone(),
                    expiration,
                },
            );
        }

        // Verify that looking up profile-a returns profile-a's credentials
        {
            let guard = cache.lock().unwrap();
            let cached_a = guard.get("profile-a").unwrap();
            assert_eq!(
                cached_a.credentials.access_key_id, "AKIAPROFILE_A_KEY",
                "Profile A should return Profile A's credentials"
            );
        }

        // Verify that looking up profile-b returns profile-b's credentials (not profile-a's)
        {
            let guard = cache.lock().unwrap();
            let cached_b = guard.get("profile-b").unwrap();
            assert_eq!(
                cached_b.credentials.access_key_id, "AKIAPROFILE_B_KEY",
                "Profile B should return Profile B's credentials, not Profile A's"
            );
        }

        // Verify that a non-existent profile returns None
        {
            let guard = cache.lock().unwrap();
            assert!(
                guard.get("profile-c").is_none(),
                "Non-existent profile should not return cached credentials"
            );
        }
    }

    #[test]
    fn test_parse_ini_file() {
        let content = r#"
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = secret_default

[profile dev]
aws_access_key_id = AKIADEV
aws_secret_access_key = secret_dev
"#;
        let sections = parse_ini_file(content);

        assert!(sections.contains_key("default"));
        assert!(sections.contains_key("dev")); // "profile " prefix stripped

        let default_section = sections.get("default").unwrap();
        assert_eq!(
            default_section.get("aws_access_key_id").unwrap(),
            "AKIADEFAULT"
        );
    }

    #[test]
    fn test_credential_process_success() {
        // We use 'echo' to simulate a credential process
        // This relies on 'echo' being available, which is true on Unix and Windows (usually).
        let json = r#"{"Version": 1, "AccessKeyId": "test_key", "SecretAccessKey": "test_secret", "SessionToken": "test_token", "Expiration": "2099-01-01T00:00:00Z"}"#;

        // Escape quotes for shell
        // On unix sh -c 'echo ...'
        // On windows cmd /C echo ...
        #[cfg(not(windows))]
        let cmd = format!("echo '{}'", json);
        #[cfg(windows)]
        let cmd = format!("echo {}", json.replace("\"", "\\\""));

        let result = execute_credential_process(&cmd);
        assert!(
            result.is_ok(),
            "credential_process failed: {:?}",
            result.err()
        );

        let (creds, exp) = result.unwrap();
        assert_eq!(creds.access_key_id, "test_key");
        assert_eq!(creds.secret_access_key, "test_secret");
        assert_eq!(creds.session_token, Some("test_token".to_string()));
        assert!(exp.is_some());
    }

    #[test]
    fn test_parse_assume_role_response() {
        let xml = r#"
        <AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
            <AssumeRoleResult>
                <Credentials>
                    <AccessKeyId>ASIATEST123</AccessKeyId>
                    <SecretAccessKey>testsecret456</SecretAccessKey>
                    <SessionToken>testsessiontoken789</SessionToken>
                    <Expiration>2099-01-15T12:00:00Z</Expiration>
                </Credentials>
                <AssumedRoleUser>
                    <AssumedRoleId>AROATEST:taws-session</AssumedRoleId>
                    <Arn>arn:aws:sts::123456789012:assumed-role/TestRole/taws-session</Arn>
                </AssumedRoleUser>
            </AssumeRoleResult>
        </AssumeRoleResponse>
        "#;

        let result = parse_assume_role_response(xml);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

        let (creds, _exp) = result.unwrap();
        assert_eq!(creds.access_key_id, "ASIATEST123");
        assert_eq!(creds.secret_access_key, "testsecret456");
        assert_eq!(creds.session_token, Some("testsessiontoken789".to_string()));
    }

    #[test]
    fn test_parse_sts_error() {
        let xml = r#"
        <ErrorResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
            <Error>
                <Code>AccessDenied</Code>
                <Message>User is not authorized to perform sts:AssumeRole</Message>
            </Error>
            <RequestId>12345678-1234-1234-1234-123456789012</RequestId>
        </ErrorResponse>
        "#;

        let result = parse_sts_error(xml);
        assert!(result.is_some());
        let error_msg = result.unwrap();
        assert!(error_msg.contains("AccessDenied"));
        assert!(error_msg.contains("not authorized"));
    }

    #[test]
    fn test_assume_role_cache_is_profile_aware() {
        let cache = ASSUME_ROLE_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

        let creds_dev = Credentials {
            access_key_id: "ASIA_DEV_KEY".to_string(),
            secret_access_key: "secret_dev".to_string(),
            session_token: Some("token_dev".to_string()),
        };
        let creds_prod = Credentials {
            access_key_id: "ASIA_PROD_KEY".to_string(),
            secret_access_key: "secret_prod".to_string(),
            session_token: Some("token_prod".to_string()),
        };

        let expiration = Instant::now() + Duration::from_secs(3600);

        {
            let mut guard = cache.lock().unwrap();
            guard.insert(
                "dev-role".to_string(),
                CachedImdsCredentials {
                    credentials: creds_dev.clone(),
                    expiration,
                },
            );
            guard.insert(
                "prod-role".to_string(),
                CachedImdsCredentials {
                    credentials: creds_prod.clone(),
                    expiration,
                },
            );
        }

        // Verify separate caching
        {
            let guard = cache.lock().unwrap();
            let cached_dev = guard.get("dev-role").unwrap();
            assert_eq!(cached_dev.credentials.access_key_id, "ASIA_DEV_KEY");

            let cached_prod = guard.get("prod-role").unwrap();
            assert_eq!(cached_prod.credentials.access_key_id, "ASIA_PROD_KEY");
        }
    }

    #[test]
    fn test_parse_ini_file_with_role_arn() {
        let content = r#"
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = secret_default
region = us-east-1

[profile production]
role_arn = arn:aws:iam::123456789012:role/ProductionAccess
source_profile = default
region = us-west-2
external_id = my-external-id

[profile staging]
role_arn = arn:aws:iam::987654321098:role/StagingAccess
source_profile = default
role_session_name = my-custom-session
duration_seconds = 7200
"#;
        let sections = parse_ini_file(content);

        // Check default profile
        assert!(sections.contains_key("default"));
        let default_section = sections.get("default").unwrap();
        assert_eq!(
            default_section.get("aws_access_key_id").unwrap(),
            "AKIADEFAULT"
        );

        // Check production profile with role_arn
        assert!(sections.contains_key("production"));
        let prod_section = sections.get("production").unwrap();
        assert_eq!(
            prod_section.get("role_arn").unwrap(),
            "arn:aws:iam::123456789012:role/ProductionAccess"
        );
        assert_eq!(prod_section.get("source_profile").unwrap(), "default");
        assert_eq!(prod_section.get("external_id").unwrap(), "my-external-id");

        // Check staging profile
        assert!(sections.contains_key("staging"));
        let staging_section = sections.get("staging").unwrap();
        assert_eq!(
            staging_section.get("role_arn").unwrap(),
            "arn:aws:iam::987654321098:role/StagingAccess"
        );
        assert_eq!(
            staging_section.get("role_session_name").unwrap(),
            "my-custom-session"
        );
        assert_eq!(staging_section.get("duration_seconds").unwrap(), "7200");
    }

    #[test]
    fn test_parse_credential_source() {
        assert_eq!(
            parse_credential_source("Environment"),
            Some(CredentialSource::Environment)
        );
        assert_eq!(
            parse_credential_source("Ec2InstanceMetadata"),
            Some(CredentialSource::Ec2InstanceMetadata)
        );
        assert_eq!(
            parse_credential_source("EcsContainer"),
            Some(CredentialSource::EcsContainer)
        );
        assert_eq!(parse_credential_source("Invalid"), None);
        assert_eq!(parse_credential_source("environment"), None); // Case sensitive
    }

    #[test]
    fn test_parse_ini_file_with_credential_source() {
        let content = r#"
[profile ecs-role]
role_arn = arn:aws:iam::123456789012:role/EcsRole
credential_source = EcsContainer
region = us-east-1

[profile ec2-role]
role_arn = arn:aws:iam::123456789012:role/Ec2Role
credential_source = Ec2InstanceMetadata

[profile env-role]
role_arn = arn:aws:iam::123456789012:role/EnvRole
credential_source = Environment
"#;
        let sections = parse_ini_file(content);

        // Check ECS profile
        assert!(sections.contains_key("ecs-role"));
        let ecs_section = sections.get("ecs-role").unwrap();
        assert_eq!(
            ecs_section.get("role_arn").unwrap(),
            "arn:aws:iam::123456789012:role/EcsRole"
        );
        assert_eq!(
            ecs_section.get("credential_source").unwrap(),
            "EcsContainer"
        );
        assert!(ecs_section.get("source_profile").is_none());

        // Check EC2 profile
        assert!(sections.contains_key("ec2-role"));
        let ec2_section = sections.get("ec2-role").unwrap();
        assert_eq!(
            ec2_section.get("credential_source").unwrap(),
            "Ec2InstanceMetadata"
        );

        // Check Environment profile
        assert!(sections.contains_key("env-role"));
        let env_section = sections.get("env-role").unwrap();
        assert_eq!(env_section.get("credential_source").unwrap(), "Environment");
    }

    #[test]
    fn test_ecs_cache() {
        let cache = ECS_CACHE.get_or_init(|| std::sync::Mutex::new(None));

        let creds = Credentials {
            access_key_id: "ASIA_ECS_KEY".to_string(),
            secret_access_key: "secret_ecs".to_string(),
            session_token: Some("token_ecs".to_string()),
        };

        let expiration = Instant::now() + Duration::from_secs(3600);

        {
            let mut guard = cache.lock().unwrap();
            *guard = Some(CachedImdsCredentials {
                credentials: creds.clone(),
                expiration,
            });
        }

        // Verify caching
        {
            let guard = cache.lock().unwrap();
            let cached = guard.as_ref().unwrap();
            assert_eq!(cached.credentials.access_key_id, "ASIA_ECS_KEY");
        }
    }

    #[test]
    fn test_try_read_cli_cache_file_matching_role() {
        // Test that try_read_cli_cache_file correctly matches role ARNs
        use std::io::Write;
        use tempfile::NamedTempFile;

        let cache_json = r#"{
            "Credentials": {
                "AccessKeyId": "ASIATESTACCESSKEY",
                "SecretAccessKey": "testsecretkey123",
                "SessionToken": "testsessiontoken456",
                "Expiration": "2099-01-01T00:00:00Z"
            },
            "AssumedRoleUser": {
                "AssumedRoleId": "AROATESTROLE:taws-session",
                "Arn": "arn:aws:sts::123456789012:assumed-role/TestRole/taws-session"
            }
        }"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(cache_json.as_bytes()).unwrap();

        // Matching role ARN
        let role_arn = "arn:aws:iam::123456789012:role/TestRole";
        let result = try_read_cli_cache_file(temp_file.path(), role_arn);
        assert!(result.is_some(), "Should find matching credentials");

        let creds = result.unwrap();
        assert_eq!(creds.access_key_id, "ASIATESTACCESSKEY");
        assert_eq!(creds.secret_access_key, "testsecretkey123");
        assert_eq!(creds.session_token, Some("testsessiontoken456".to_string()));
    }

    #[test]
    fn test_try_read_cli_cache_file_non_matching_role() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let cache_json = r#"{
            "Credentials": {
                "AccessKeyId": "ASIATESTACCESSKEY",
                "SecretAccessKey": "testsecretkey123",
                "SessionToken": "testsessiontoken456",
                "Expiration": "2099-01-01T00:00:00Z"
            },
            "AssumedRoleUser": {
                "AssumedRoleId": "AROATESTROLE:taws-session",
                "Arn": "arn:aws:sts::123456789012:assumed-role/TestRole/taws-session"
            }
        }"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(cache_json.as_bytes()).unwrap();

        // Non-matching role ARN (different role name)
        let role_arn = "arn:aws:iam::123456789012:role/DifferentRole";
        let result = try_read_cli_cache_file(temp_file.path(), role_arn);
        assert!(
            result.is_none(),
            "Should not find credentials for different role"
        );

        // Non-matching role ARN (different account)
        let role_arn = "arn:aws:iam::999999999999:role/TestRole";
        let result = try_read_cli_cache_file(temp_file.path(), role_arn);
        assert!(
            result.is_none(),
            "Should not find credentials for different account"
        );
    }

    #[test]
    fn test_try_read_cli_cache_file_expired() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let cache_json = r#"{
            "Credentials": {
                "AccessKeyId": "ASIATESTACCESSKEY",
                "SecretAccessKey": "testsecretkey123",
                "SessionToken": "testsessiontoken456",
                "Expiration": "2020-01-01T00:00:00Z"
            },
            "AssumedRoleUser": {
                "AssumedRoleId": "AROATESTROLE:taws-session",
                "Arn": "arn:aws:sts::123456789012:assumed-role/TestRole/taws-session"
            }
        }"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(cache_json.as_bytes()).unwrap();

        let role_arn = "arn:aws:iam::123456789012:role/TestRole";
        let result = try_read_cli_cache_file(temp_file.path(), role_arn);
        assert!(result.is_none(), "Should not return expired credentials");
    }
}
