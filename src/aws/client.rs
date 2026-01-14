//! AWS Client - Lightweight HTTP-based client
//!
//! Uses SigV4 signing with direct HTTP calls instead of heavy SDK

use anyhow::Result;

use super::credentials::{load_credentials, load_credentials_with_sso_check, CredentialsError};
use super::http::AwsHttpClient;

/// Result type for client creation that may require SSO login
pub enum ClientResult {
    /// Client created successfully
    Ok(AwsClients, String),
    /// SSO login required before client can be created
    SsoLoginRequired { profile: String, sso_session: String, region: String, endpoint_url: Option<String> },
}

/// Container for AWS HTTP client
pub struct AwsClients {
    pub http: AwsHttpClient,
    pub region: String,
    pub profile: String,
}

impl AwsClients {
    /// Create AWS client for a given profile and region
    /// Note: This runs credential loading on a blocking thread to support SSO
    pub async fn new(profile: &str, region: &str, endpoint_url: Option<String>) -> Result<(Self, String)> {
        let profile_str = profile.to_string();
        let region_str = region.to_string();
        let profile_for_closure = profile_str.clone();
        
        // Run credential loading on blocking thread (SSO uses blocking HTTP)
        let credentials = tokio::task::spawn_blocking(move || {
            load_credentials(&profile_for_closure)
        }).await??;
        
        let http = AwsHttpClient::new(credentials, &region_str, endpoint_url);

        let client = Self {
            http,
            region: region_str.clone(),
            profile: profile_str,
        };

        Ok((client, region_str))
    }
    
    /// Create AWS client with SSO check - returns specific error if SSO login is needed
    /// Note: This runs credential loading on a blocking thread to support SSO
    pub async fn new_with_sso_check(profile: &str, region: &str, endpoint_url: Option<String>) -> Result<ClientResult> {
        let profile = profile.to_string();
        let region = region.to_string();
        let endpoint = endpoint_url.clone();
        
        // Run credential loading on blocking thread (SSO uses blocking HTTP)
        let cred_result = tokio::task::spawn_blocking(move || {
            load_credentials_with_sso_check(&profile)
                .map(|c| (c, profile))
        }).await?;
        
        match cred_result {
            Ok((credentials, prof)) => {
                let http = AwsHttpClient::new(credentials, &region, endpoint_url);
                let client = Self {
                    http,
                    region: region.clone(),
                    profile: prof,
                };
                Ok(ClientResult::Ok(client, region))
            }
            Err(CredentialsError::SsoLoginRequired { profile, sso_session }) => {
                Ok(ClientResult::SsoLoginRequired { 
                    profile, 
                    sso_session, 
                    region,
                    endpoint_url: endpoint,
                })
            }
            Err(CredentialsError::Other(e)) => Err(e),
        }
    }

    /// Recreate client for a new region (keeps same profile)
    /// Note: This runs credential loading on a blocking thread to support SSO
    pub async fn switch_region(&mut self, profile: &str, region: &str) -> Result<String> {
        let profile_str = profile.to_string();
        let region_str = region.to_string();
        let profile_for_closure = profile_str.clone();
        
        // Run credential loading on blocking thread (SSO uses blocking HTTP)
        let credentials = tokio::task::spawn_blocking(move || {
            load_credentials(&profile_for_closure)
        }).await??;
        
        self.http.set_credentials(credentials);
        self.http.set_region(&region_str);
        self.region = region_str.clone();
        self.profile = profile_str;
        Ok(region_str)
    }
}

/// Format AWS errors into user-friendly messages
pub fn format_aws_error(err: &anyhow::Error) -> String {
    let err_str = err.to_string();
    
    // Check for common AWS error patterns
    if err_str.contains("dispatch failure") || err_str.contains("connection") {
        return "Connection failed - check internet/credentials".to_string();
    }
    if err_str.contains("InvalidClientTokenId") || err_str.contains("SignatureDoesNotMatch") {
        return "Invalid credentials - run 'aws configure'".to_string();
    }
    if err_str.contains("ExpiredToken") {
        return "Credentials expired - refresh or reconfigure".to_string();
    }
    if err_str.contains("AccessDenied") || err_str.contains("UnauthorizedAccess") {
        return "Access denied - check IAM permissions".to_string();
    }
    if err_str.contains("No credentials") || err_str.contains("no credentials") {
        return "No credentials - run 'aws configure'".to_string();
    }
    if err_str.contains("timeout") || err_str.contains("Timeout") {
        return "Request timed out - check connection".to_string();
    }
    if err_str.contains("not available yet") {
        return err_str;
    }
    if err_str.contains("region") {
        return "Region error - check AWS_REGION".to_string();
    }
    
    // Default: truncate long errors
    if err_str.len() > 60 {
        format!("{}...", &err_str[..60])
    } else {
        err_str
    }
}
