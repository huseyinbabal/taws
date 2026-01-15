//! AWS Credentials loading from multiple sources
//!
//! Supports:
//! - Environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN)
//! - AWS profiles (~/.aws/credentials and ~/.aws/config)
//! - AWS SSO (IAM Identity Center) via cached tokens
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

    // 2. Check if profile uses AssumeRole (role_arn + source_profile)
    if let Ok(creds) = load_with_assume_role(profile) {
        debug!("Loaded credentials via AssumeRole for profile '{}'", profile);
        return Ok(creds);
    }

    // 3. Check if SSO is configured for this profile - if so, prioritize SSO
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

    // 4. Try AWS credentials file
    if let Ok(creds) = load_from_credentials_file(profile) {
        debug!(
            "Loaded credentials from credentials file for profile '{}'",
            profile
        );
        return Ok(creds);
    }

    // 5. Try config file with direct credentials
    if let Ok(creds) = load_from_config_file(profile) {
        debug!(
            "Loaded credentials from config file for profile '{}'",
            profile
        );
        return Ok(creds);
    }

    // 6. Try IMDSv2 (EC2 instance metadata) - only for default profile
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
    let config_path = aws_config_dir()?.join("config");
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
// AssumeRole Support (AWS CLI Cache)
// =============================================================================

/// Load credentials from AWS CLI cache for profiles with role_arn and source_profile
/// The AWS CLI caches assumed role credentials in ~/.aws/cli/cache/
fn load_with_assume_role(profile: &str) -> Result<Credentials> {
    let config_path = aws_config_dir()?.join("config");
    let content = fs::read_to_string(&config_path)
        .map_err(|_| anyhow!("Could not read config file"))?;

    let sections = parse_ini_file(&content);
    let section = sections
        .get(profile)
        .ok_or_else(|| anyhow!("Profile '{}' not found", profile))?;

    // Check if this profile uses AssumeRole
    let _role_arn = section
        .get("role_arn")
        .ok_or_else(|| anyhow!("No role_arn configured"))?;

    let _source_profile = section
        .get("source_profile")
        .ok_or_else(|| anyhow!("No source_profile configured"))?;

    debug!(
        "Profile '{}' uses AssumeRole, checking AWS CLI cache",
        profile
    );

    // Try to read from AWS CLI cache
    read_cli_cache(profile)
}

/// Read cached credentials from AWS CLI cache directory
/// AWS CLI stores assumed role credentials in ~/.aws/cli/cache/<hash>.json
fn read_cli_cache(profile: &str) -> Result<Credentials> {
    let cache_dir = aws_config_dir()?.join("cli").join("cache");
    
    if !cache_dir.exists() {
        return Err(anyhow!("AWS CLI cache directory not found. Please run 'aws' command with profile '{}' first.", profile));
    }

    // Get role_arn from profile to match against cache files
    let config_path = aws_config_dir()?.join("config");
    let content = fs::read_to_string(&config_path)
        .map_err(|_| anyhow!("Could not read config file"))?;

    let sections = parse_ini_file(&content);
    let section = sections
        .get(profile)
        .ok_or_else(|| anyhow!("Profile '{}' not found", profile))?;

    let role_arn = section
        .get("role_arn")
        .ok_or_else(|| anyhow!("No role_arn configured"))?;

    trace!("Looking for cached credentials for role_arn: {}", role_arn);

    // Search through all cache files to find one matching this role_arn
    let entries = fs::read_dir(&cache_dir)
        .map_err(|e| anyhow!("Failed to read cache directory: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| anyhow!("Failed to read directory entry: {}", e))?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        // Read and parse cache file
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue, // Skip files we can't read
        };

        let cache_data: serde_json::Value = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(_) => continue, // Skip invalid JSON
        };

        // Check if this cache file matches our role_arn
        if let Some(assumed_role_arn) = cache_data
            .get("AssumedRoleUser")
            .and_then(|u| u.get("Arn"))
            .and_then(|a| a.as_str())
        {
            // Extract role name from both ARNs and compare
            // AssumedRoleUser ARN format: arn:aws:sts::account-id:assumed-role/role-name/session-name
            // role_arn format: arn:aws:iam::account-id:role/role-name
            let cache_role_name = assumed_role_arn
                .split('/')
                .nth(1)
                .unwrap_or("");
            let config_role_name = role_arn
                .split('/')
                .last()
                .unwrap_or("");

            let cache_account = assumed_role_arn
                .split(':')
                .nth(4)
                .unwrap_or("");
            let config_account = role_arn
                .split(':')
                .nth(4)
                .unwrap_or("");

            if cache_role_name == config_role_name && cache_account == config_account {
                debug!("Found matching cache file: {:?}", path);

                // Extract credentials
                let creds = cache_data
                    .get("Credentials")
                    .ok_or_else(|| anyhow!("Credentials not found in cache"))?;

                let access_key_id = creds
                    .get("AccessKeyId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("AccessKeyId not found"))?
                    .to_string();

                let secret_access_key = creds
                    .get("SecretAccessKey")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("SecretAccessKey not found"))?
                    .to_string();

                let session_token = creds
                    .get("SessionToken")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                // Check expiration
                if let Some(expiration_str) = creds.get("Expiration").and_then(|v| v.as_str()) {
                    if let Ok(expiration) = chrono::DateTime::parse_from_rfc3339(expiration_str) {
                        if expiration <= chrono::Utc::now() {
                            return Err(anyhow!("Cached credentials for profile '{}' have expired. Please run 'aws sts get-caller-identity --profile {}' to refresh.", profile, profile));
                        }
                        debug!("Using cached credentials valid until: {}", expiration);
                    }
                }

                return Ok(Credentials {
                    access_key_id,
                    secret_access_key,
                    session_token,
                });
            }
        }
    }

    Err(anyhow!(
        "No cached credentials found for profile '{}'. Please run 'aws sts get-caller-identity --profile {}' first.",
        profile,
        profile
    ))
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
}
