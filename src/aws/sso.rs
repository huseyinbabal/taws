//! AWS SSO (IAM Identity Center) OIDC Device Authorization Flow
//!
//! Implements the OAuth 2.0 Device Authorization Grant flow for AWS SSO:
//! 1. Register client with OIDC
//! 2. Start device authorization
//! 3. Open browser for user authentication
//! 4. Poll for token completion
//! 5. Cache the access token

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs;
use std::time::{Duration, SystemTime};
use tracing::{debug, trace};

use super::credentials::{aws_config_dir, Credentials};

/// SSO configuration parsed from profile
#[derive(Debug, Clone)]
pub struct SsoConfig {
    pub sso_session: String,
    pub sso_account_id: String,
    pub sso_role_name: String,
    pub sso_start_url: String,
    pub sso_region: String,
}

/// OIDC client registration response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientRegistration {
    client_id: String,
    client_secret: String,
    client_secret_expires_at: i64,
}

/// Device authorization response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub interval: i64,
}

/// Token response from OIDC
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    expires_in: i64,
}

/// Cached SSO token format (compatible with AWS CLI v1 and v2)
/// Supports both snake_case (v1/legacy) and camelCase (v2) field names via aliases
#[derive(Debug, Serialize, Deserialize)]
struct CachedToken {
    #[serde(alias = "accessToken")]
    access_token: String,
    #[serde(alias = "expiresAt")]
    expires_at: String,
    #[serde(default, alias = "region")]
    region: Option<String>,
    #[serde(alias = "startUrl")]
    start_url: String,
}

/// SSO login state for UI (kept for potential future use)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SsoLoginState {
    /// Prompt user to start SSO login
    Prompt { config: SsoConfig },
    /// Waiting for browser authentication
    WaitingForAuth {
        config: SsoConfig,
        device_auth: DeviceAuthInfo,
    },
    /// Login successful
    Success,
    /// Login failed
    Failed { error: String },
}

/// Device authorization info (subset for UI)
#[derive(Debug, Clone)]
pub struct DeviceAuthInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub device_code: String,
    pub interval: i64,
    #[allow(dead_code)]
    pub expires_at: SystemTime,
}

/// Check if we already have a valid cached token (e.g., from AWS CLI login)
/// Returns the token if valid, None otherwise
pub fn check_existing_token(config: &SsoConfig) -> Option<String> {
    read_cached_token(config)
}

/// Start the SSO OIDC device authorization flow
/// Returns device authorization info for UI display
pub fn start_device_authorization(config: &SsoConfig) -> Result<DeviceAuthInfo> {
    let client = super::tls::create_blocking_client_with_timeout(Duration::from_secs(30))?;

    let oidc_endpoint = format!("https://oidc.{}.amazonaws.com", config.sso_region);

    // Step 1: Register client
    debug!("Registering OIDC client");
    let register_url = format!("{}/client/register", oidc_endpoint);
    let register_response = client
        .post(&register_url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "clientName": "taws",
            "clientType": "public",
        }))
        .send()?;

    if !register_response.status().is_success() {
        let status = register_response.status();
        let body = register_response.text().unwrap_or_default();
        return Err(anyhow!(
            "OIDC client registration failed ({}): {}",
            status,
            body
        ));
    }

    let registration: ClientRegistration = register_response.json()?;
    trace!("Got client_id: {}", registration.client_id);

    // Step 2: Start device authorization
    debug!("Starting device authorization");
    let device_auth_url = format!("{}/device_authorization", oidc_endpoint);
    let device_response = client
        .post(&device_auth_url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "clientId": registration.client_id,
            "clientSecret": registration.client_secret,
            "startUrl": config.sso_start_url,
        }))
        .send()?;

    if !device_response.status().is_success() {
        let status = device_response.status();
        let body = device_response.text().unwrap_or_default();
        return Err(anyhow!(
            "Device authorization failed ({}): {}",
            status,
            body
        ));
    }

    let device_auth: DeviceAuthorization = device_response.json()?;
    debug!(
        "Got user_code: {}, verification_uri: {}",
        device_auth.user_code, device_auth.verification_uri
    );

    // Store client registration for token polling
    let cache_dir = aws_config_dir()?.join("sso").join("cache");
    fs::create_dir_all(&cache_dir)?;

    let client_cache_path = cache_dir.join(format!("{}_client.json", config.sso_session));
    let client_data = serde_json::json!({
        "clientId": registration.client_id,
        "clientSecret": registration.client_secret,
        "clientSecretExpiresAt": registration.client_secret_expires_at,
        "deviceCode": device_auth.device_code,
        "region": config.sso_region,
    });
    fs::write(
        &client_cache_path,
        serde_json::to_string_pretty(&client_data)?,
    )?;

    let expires_at = SystemTime::now() + Duration::from_secs(device_auth.expires_in as u64);

    Ok(DeviceAuthInfo {
        user_code: device_auth.user_code,
        verification_uri: device_auth.verification_uri,
        verification_uri_complete: device_auth.verification_uri_complete,
        device_code: device_auth.device_code,
        interval: device_auth.interval,
        expires_at,
    })
}

/// Open browser to SSO login page
pub fn open_sso_browser(verification_uri_complete: &str) -> Result<()> {
    debug!("Opening browser to: {}", verification_uri_complete);
    open::that(verification_uri_complete).map_err(|e| anyhow!("Failed to open browser: {}", e))
}

/// Poll for token completion (call this periodically)
/// Returns Ok(Some(token)) when authorized, Ok(None) when still pending, Err on failure
pub fn poll_for_token(config: &SsoConfig) -> Result<Option<String>> {
    let cache_dir = aws_config_dir()?.join("sso").join("cache");
    let client_cache_path = cache_dir.join(format!("{}_client.json", config.sso_session));

    let client_data: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&client_cache_path)
            .map_err(|_| anyhow!("Client registration not found"))?,
    )?;

    let client_id = client_data
        .get("clientId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("clientId not found"))?;
    let client_secret = client_data
        .get("clientSecret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("clientSecret not found"))?;
    let device_code = client_data
        .get("deviceCode")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("deviceCode not found"))?;

    let http_client = super::tls::create_blocking_client_with_timeout(Duration::from_secs(10))?;

    let oidc_endpoint = format!("https://oidc.{}.amazonaws.com", config.sso_region);
    let token_url = format!("{}/token", oidc_endpoint);

    trace!("Polling for token");
    let response = http_client
        .post(&token_url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "clientId": client_id,
            "clientSecret": client_secret,
            "deviceCode": device_code,
            "grantType": "urn:ietf:params:oauth:grant-type:device_code",
        }))
        .send()?;

    if response.status().is_success() {
        let token_response: TokenResponse = response.json()?;

        // Cache the token
        cache_sso_token(
            config,
            &token_response.access_token,
            token_response.expires_in,
        )?;

        // Clean up client cache
        let _ = fs::remove_file(&client_cache_path);

        debug!("SSO authentication successful");
        return Ok(Some(token_response.access_token));
    }

    // Check for authorization_pending (still waiting)
    let body = response.text().unwrap_or_default();
    if body.contains("authorization_pending") || body.contains("AuthorizationPendingException") {
        trace!("Authorization still pending");
        return Ok(None);
    }

    // Check for slow_down
    if body.contains("slow_down") || body.contains("SlowDownException") {
        trace!("Slow down requested");
        return Ok(None);
    }

    // Check for expired
    if body.contains("expired") || body.contains("ExpiredTokenException") {
        return Err(anyhow!("SSO authorization expired. Please try again."));
    }

    // Other error
    Err(anyhow!("Token request failed: {}", body))
}

/// Cache the SSO access token (compatible with AWS CLI format)
fn cache_sso_token(config: &SsoConfig, access_token: &str, expires_in: i64) -> Result<()> {
    let cache_dir = aws_config_dir()?.join("sso").join("cache");
    fs::create_dir_all(&cache_dir)?;

    // Calculate expiration time
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);
    let expires_at_str = expires_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let cached_token = CachedToken {
        access_token: access_token.to_string(),
        expires_at: expires_at_str,
        region: Some(config.sso_region.clone()),
        start_url: config.sso_start_url.clone(),
    };

    // Cache file name is SHA1 of start_url (compatible with AWS CLI for both legacy and new format)
    let mut hasher = Sha1::new();
    hasher.update(config.sso_start_url.as_bytes());
    let hash = hasher.finalize();
    let cache_file_name = format!("{:x}.json", hash);
    let cache_path = cache_dir.join(&cache_file_name);

    fs::write(&cache_path, serde_json::to_string_pretty(&cached_token)?)?;
    debug!("Cached SSO token to {:?}", cache_path);

    Ok(())
}

/// Get role credentials using SSO access token
pub fn get_role_credentials(config: &SsoConfig, access_token: &str) -> Result<Credentials> {
    let client = super::tls::create_blocking_client_with_timeout(Duration::from_secs(10))?;

    let url = format!(
        "https://portal.sso.{}.amazonaws.com/federation/credentials",
        config.sso_region
    );

    trace!("Fetching role credentials from: {}", url);

    let response = client
        .get(&url)
        .query(&[
            ("account_id", &config.sso_account_id),
            ("role_name", &config.sso_role_name),
        ])
        .header("x-amz-sso_bearer_token", access_token)
        .send()?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("GetRoleCredentials failed ({}): {}", status, body));
    }

    let json: serde_json::Value = response.json()?;
    let role_creds = json
        .get("roleCredentials")
        .ok_or_else(|| anyhow!("roleCredentials not found"))?;

    let access_key_id = role_creds
        .get("accessKeyId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("accessKeyId not found"))?
        .to_string();

    let secret_access_key = role_creds
        .get("secretAccessKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("secretAccessKey not found"))?
        .to_string();

    let session_token = role_creds
        .get("sessionToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(Credentials {
        access_key_id,
        secret_access_key,
        session_token,
    })
}

/// Check if SSO is configured for a profile and return config if so
pub fn get_sso_config(profile: &str) -> Option<SsoConfig> {
    let config_path = aws_config_dir().ok()?.join("config");
    let content = fs::read_to_string(&config_path).ok()?;

    parse_sso_config_from_content(profile, &content).ok()
}

/// Parse SSO config from content
/// Supports both new format (sso_session reference) and legacy format (direct sso_start_url)
fn parse_sso_config_from_content(profile: &str, content: &str) -> Result<SsoConfig> {
    let sections = parse_ini_sections(content);

    let profile_section = sections
        .get(profile)
        .ok_or_else(|| anyhow!("Profile '{}' not found", profile))?;

    // Check for required fields that exist in both formats
    let sso_account_id = profile_section
        .get("sso_account_id")
        .ok_or_else(|| anyhow!("No sso_account_id in profile"))?
        .clone();

    let sso_role_name = profile_section
        .get("sso_role_name")
        .ok_or_else(|| anyhow!("No sso_role_name in profile"))?
        .clone();

    // Try new format first (sso_session reference)
    if let Some(sso_session) = profile_section.get("sso_session") {
        let session_key = format!("sso-session {}", sso_session);
        let session_section = sections
            .get(&session_key)
            .ok_or_else(|| anyhow!("SSO session '{}' not found", sso_session))?;

        let sso_start_url = session_section
            .get("sso_start_url")
            .ok_or_else(|| anyhow!("No sso_start_url in session"))?
            .clone();

        let sso_region = session_section
            .get("sso_region")
            .ok_or_else(|| anyhow!("No sso_region in session"))?
            .clone();

        return Ok(SsoConfig {
            sso_session: sso_session.clone(),
            sso_account_id,
            sso_role_name,
            sso_start_url,
            sso_region,
        });
    }

    // Fall back to legacy format (sso_start_url directly in profile)
    let sso_start_url = profile_section
        .get("sso_start_url")
        .ok_or_else(|| anyhow!("No sso_start_url or sso_session in profile"))?
        .clone();

    let sso_region = profile_section
        .get("sso_region")
        .ok_or_else(|| anyhow!("No sso_region in profile"))?
        .clone();

    // For legacy format, use profile name as session identifier
    Ok(SsoConfig {
        sso_session: profile.to_string(),
        sso_account_id,
        sso_role_name,
        sso_start_url,
        sso_region,
    })
}

/// Parse INI file into sections (duplicated here to avoid circular deps)
fn parse_ini_sections(
    content: &str,
) -> std::collections::HashMap<String, std::collections::HashMap<String, String>> {
    let mut sections = std::collections::HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_string();
            if current_section.starts_with("profile ") {
                current_section = current_section["profile ".len()..].to_string();
            }
            sections
                .entry(current_section.clone())
                .or_insert_with(std::collections::HashMap::new);
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            if !current_section.is_empty() {
                sections
                    .entry(current_section.clone())
                    .or_insert_with(std::collections::HashMap::new)
                    .insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    sections
}

/// Read cached SSO token if valid
/// Tries multiple cache file formats for compatibility with AWS CLI v1 and v2
pub fn read_cached_token(config: &SsoConfig) -> Option<String> {
    let cache_dir = aws_config_dir().ok()?.join("sso").join("cache");

    // AWS CLI v2 with sso_session uses SHA1 of the session name for cache file
    // Try this format first as it's the most current
    let mut hasher = Sha1::new();
    hasher.update(config.sso_session.as_bytes());
    let hash = hasher.finalize();
    let cache_file_v2 = format!("{:x}.json", hash);
    let cache_path_v2 = cache_dir.join(&cache_file_v2);

    if let Some(token) = try_read_token_file(&cache_path_v2) {
        debug!(
            "Found valid SSO token using CLI v2 format (sso_session: {})",
            config.sso_session
        );
        return Some(token);
    }

    // AWS CLI v1 / legacy format uses SHA1 of start_url for cache file
    let mut hasher = Sha1::new();
    hasher.update(config.sso_start_url.as_bytes());
    let hash = hasher.finalize();
    let cache_file_legacy = format!("{:x}.json", hash);
    let cache_path_legacy = cache_dir.join(&cache_file_legacy);

    if let Some(token) = try_read_token_file(&cache_path_legacy) {
        debug!("Found valid SSO token using legacy format (start_url-based)");
        return Some(token);
    }

    trace!(
        "No valid SSO token found in cache for session '{}' or start_url",
        config.sso_session
    );
    None
}

/// Helper function to read and validate a token file
fn try_read_token_file(cache_path: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(cache_path).ok()?;
    let cached: CachedToken = serde_json::from_str(&content).ok()?;

    // Check expiration
    if let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(&cached.expires_at) {
        if expires_at <= chrono::Utc::now() {
            trace!("SSO token in {:?} is expired", cache_path);
            return None;
        }
    }

    Some(cached.access_token)
}
