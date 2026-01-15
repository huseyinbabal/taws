//! AWS API Dispatcher
//!
//! This module handles all AWS API dispatching:
//! - List operations via JSON config
//! - Actions (write operations like start/stop/delete)
//! - Describe (single resource details)
//!
//! API operations are configured in JSON files under src/resources/.
//! Special cases (S3 objects, STS) have dedicated handlers.

use super::field_mapper::build_response;
use super::handlers::get_protocol_handler;
use super::protocol::ApiProtocol;
use super::registry::get_resource;
use crate::aws::client::AwsClients;
use crate::aws::http::xml_to_json;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tracing::debug;

// =============================================================================
// Helper Functions
// =============================================================================

/// Extract a single string parameter from Value
fn extract_param(params: &Value, key: &str) -> String {
    params
        .get(key)
        .and_then(|v| {
            v.as_str().map(|s| s.to_string()).or_else(|| {
                v.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
        })
        .unwrap_or_default()
}

/// Format bytes into human-readable format
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format epoch milliseconds to human-readable date string
fn format_epoch_millis(millis: i64) -> String {
    use chrono::{TimeZone, Utc};

    if millis <= 0 {
        return "-".to_string();
    }

    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Format epoch milliseconds to human-readable date string (public for log tail UI)
pub fn format_log_timestamp(millis: i64) -> String {
    format_epoch_millis(millis)
}

// =============================================================================
// Data-Driven List Operations
// =============================================================================

/// Invoke an AWS list API using JSON configuration
///
/// This function reads the API configuration from the resource definition
/// and uses the appropriate protocol handler to execute the request.
pub async fn invoke_list(
    resource_key: &str,
    clients: &AwsClients,
    params: &Value,
) -> Result<Value> {
    let resource_def =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    let api_config = resource_def
        .api_config
        .as_ref()
        .ok_or_else(|| anyhow!("Resource {} does not have api_config", resource_key))?;

    let handler = get_protocol_handler(api_config.protocol);

    let service = api_config
        .service_name
        .as_deref()
        .unwrap_or(&resource_def.service);

    let parsed = handler
        .invoke(
            clients,
            service,
            api_config,
            params,
            &resource_def.field_mappings,
        )
        .await?;

    Ok(build_response(
        parsed.items,
        &resource_def.response_path,
        parsed.next_token,
    ))
}

/// Check if a resource has API config for list operations
pub fn has_api_config(resource_key: &str) -> bool {
    get_resource(resource_key)
        .map(|r| r.api_config.is_some())
        .unwrap_or(false)
}

/// Get the protocol type for a resource
pub fn get_resource_protocol(resource_key: &str) -> Option<ApiProtocol> {
    get_resource(resource_key)
        .and_then(|r| r.api_config.as_ref())
        .map(|c| c.protocol)
}

// =============================================================================
// Legacy List Operations (special cases)
// =============================================================================

/// Invoke an AWS API method for special cases (S3 objects, STS, CloudWatch logs).
/// Most list operations should use invoke_list instead.
pub async fn invoke_sdk(
    service: &str,
    method: &str,
    clients: &AwsClients,
    params: &Value,
) -> Result<Value> {
    match (service, method) {
        // S3 list_objects_v2 - requires bucket region resolution and complex folder handling
        ("s3", "list_objects_v2") => {
            let bucket = params
                .get("bucket_names")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Bucket name required"))?;

            let prefix = params
                .get("prefix")
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else if let Some(arr) = v.as_array() {
                        arr.first()
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();

            let bucket_region = clients.http.get_bucket_region(bucket).await?;
            debug!("Bucket {} is in region {}", bucket, bucket_region);

            let path = if prefix.is_empty() {
                "?list-type=2&delimiter=/".to_string()
            } else {
                format!(
                    "?list-type=2&delimiter=/&prefix={}",
                    urlencoding::encode(&prefix)
                )
            };

            let xml = clients
                .http
                .rest_xml_request_s3_bucket("GET", bucket, &path, None, &bucket_region)
                .await?;
            let json = xml_to_json(&xml)?;

            let mut objects: Vec<Value> = vec![];

            // Add common prefixes (folders)
            if let Some(prefixes) = json.pointer("/ListBucketResult/CommonPrefixes") {
                let prefix_list = match prefixes {
                    Value::Array(arr) => arr.clone(),
                    obj @ Value::Object(_) => vec![obj.clone()],
                    _ => vec![],
                };
                for p in prefix_list {
                    let prefix_val = p.pointer("/Prefix").and_then(|v| v.as_str()).unwrap_or("-");
                    let display_name = prefix_val
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(prefix_val);
                    objects.push(json!({
                        "Key": prefix_val,
                        "DisplayName": format!("{}/", display_name),
                        "Size": "-",
                        "LastModified": "-",
                        "StorageClass": "FOLDER",
                        "IsFolder": true
                    }));
                }
            }

            // Add objects (files)
            if let Some(contents) = json.pointer("/ListBucketResult/Contents") {
                let content_list = match contents {
                    Value::Array(arr) => arr.clone(),
                    obj @ Value::Object(_) => vec![obj.clone()],
                    _ => vec![],
                };
                for obj in content_list {
                    let key = obj.pointer("/Key").and_then(|v| v.as_str()).unwrap_or("-");
                    if key == prefix {
                        continue;
                    }
                    let display_name = key.rsplit('/').next().unwrap_or(key);
                    let size = obj.pointer("/Size").and_then(|v| v.as_str()).unwrap_or("0");
                    let size_formatted = format_bytes(size.parse::<u64>().unwrap_or(0));
                    objects.push(json!({
                        "Key": key,
                        "DisplayName": display_name,
                        "Size": size_formatted,
                        "LastModified": obj.pointer("/LastModified").and_then(|v| v.as_str()).unwrap_or("-"),
                        "StorageClass": obj.pointer("/StorageClass").and_then(|v| v.as_str()).unwrap_or("STANDARD"),
                        "IsFolder": false
                    }));
                }
            }

            Ok(json!({ "objects": objects }))
        }

        // STS get_caller_identity - returns single item, not a list
        ("sts", "get_caller_identity") => {
            let xml = clients
                .http
                .query_request("sts", "GetCallerIdentity", &[])
                .await?;
            let json = xml_to_json(&xml)?;

            let result_path = json.pointer("/GetCallerIdentityResponse/GetCallerIdentityResult");
            let identity = json!({
                "Account": result_path.and_then(|r| r.pointer("/Account")).and_then(|v| v.as_str()).unwrap_or("-"),
                "UserId": result_path.and_then(|r| r.pointer("/UserId")).and_then(|v| v.as_str()).unwrap_or("-"),
                "Arn": result_path.and_then(|r| r.pointer("/Arn")).and_then(|v| v.as_str()).unwrap_or("-"),
            });

            Ok(json!({ "identity": [identity] }))
        }

        // CloudWatch Logs - tail_logs (streaming operation)
        ("cloudwatchlogs", "tail_logs") => {
            let log_group = extract_param(params, "log_group_name");
            let log_stream = extract_param(params, "log_stream_name");

            if log_group.is_empty() || log_stream.is_empty() {
                return Err(anyhow!("Log group and stream names required"));
            }

            let request_body = json!({
                "logGroupName": log_group,
                "logStreamName": log_stream,
                "startFromHead": false,
                "limit": 100
            })
            .to_string();

            let response = clients
                .http
                .json_request("logs", "GetLogEvents", &request_body)
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            let events = json
                .get("events")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let result: Vec<Value> = events
                .iter()
                .map(|e| {
                    let timestamp = e.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
                    json!({
                        "timestamp": format_epoch_millis(timestamp),
                        "message": e.get("message").and_then(|v| v.as_str()).unwrap_or("-"),
                    })
                })
                .collect();

            Ok(json!({ "events": result }))
        }

        // CloudWatch Logs - get_log_events (for log tailing UI)
        ("cloudwatchlogs", "get_log_events") => {
            let log_group = extract_param(params, "log_group_name");
            let log_stream = extract_param(params, "log_stream_name");

            if log_group.is_empty() || log_stream.is_empty() {
                return Err(anyhow!("Log group and stream names required"));
            }

            let mut request = json!({
                "logGroupName": log_group,
                "logStreamName": log_stream,
                "startFromHead": false,
                "limit": 100
            });

            // Add next token if provided
            if let Some(token) = params.get("next_forward_token").and_then(|v| v.as_str()) {
                request["nextToken"] = json!(token);
            }

            let response = clients
                .http
                .json_request("logs", "GetLogEvents", &request.to_string())
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            Ok(json)
        }

        _ => Err(anyhow!(
            "Operation not handled: service='{}', method='{}'. Configure it in the resource JSON.",
            service,
            method
        )),
    }
}

// =============================================================================
// Data-Driven Action Execution
// =============================================================================

/// Execute an action using JSON configuration
async fn invoke_action(
    resource_key: &str,
    action_id: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<()> {
    let resource_def =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    let action_config = resource_def
        .action_configs
        .get(action_id)
        .ok_or_else(|| anyhow!("Action '{}' not configured for {}", action_id, resource_key))?;

    let service = action_config
        .service_name
        .as_deref()
        .unwrap_or(&resource_def.service);

    debug!(
        "Executing action: {} on {} (service: {}, protocol: {:?})",
        action_id, resource_key, service, action_config.protocol
    );

    match action_config.protocol {
        ApiProtocol::Query => {
            let action_name = action_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("Query action requires 'action' field"))?;

            let mut params_owned: Vec<(String, String)> = Vec::new();

            // Handle special formats
            if action_config.special_handling.as_deref() == Some("parse_pipe_format_tg_target") {
                // Format: target_group_arn|target_id
                let parts: Vec<&str> = resource_id.split('|').collect();
                if parts.len() != 2 {
                    return Err(anyhow!(
                        "Invalid target format, expected target_group_arn|target_id"
                    ));
                }
                params_owned.push(("TargetGroupArn".to_string(), parts[0].to_string()));
                params_owned.push(("Targets.member.1.Id".to_string(), parts[1].to_string()));
            } else {
                // Add resource ID parameter
                if let Some(ref id_param) = action_config.id_param {
                    params_owned.push((id_param.clone(), resource_id.to_string()));
                }
            }

            // Add static parameters
            for (key, value) in &action_config.static_params {
                if let Some(s) = value.as_str() {
                    params_owned.push((key.clone(), s.to_string()));
                }
            }

            let params_ref: Vec<(&str, &str)> = params_owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            clients
                .http
                .query_request(service, action_name, &params_ref)
                .await?;
            Ok(())
        }

        ApiProtocol::Json => {
            let action_name = action_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("JSON action requires 'action' field"))?;

            let body = if let Some(ref template) = action_config.body_template {
                // Handle special ARN parsing if needed
                let actual_id =
                    if action_config.special_handling.as_deref() == Some("parse_arn_for_cluster") {
                        // Extract cluster from ARN like arn:aws:ecs:region:account:service/cluster/service-name
                        let parts: Vec<&str> = resource_id.split('/').collect();
                        if parts.len() >= 2 {
                            parts[parts.len() - 2].to_string()
                        } else {
                            resource_id.to_string()
                        }
                    } else {
                        resource_id.to_string()
                    };

                template
                    .replace("{resource_id}", &actual_id)
                    .replace("{cluster}", &{
                        let parts: Vec<&str> = resource_id.split('/').collect();
                        if parts.len() >= 2 {
                            parts[parts.len() - 2]
                        } else {
                            resource_id
                        }
                    })
            } else {
                // Build body from id_param
                let id_param = action_config.id_param.as_deref().unwrap_or("id");
                json!({ id_param: resource_id }).to_string()
            };

            clients
                .http
                .json_request(service, action_name, &body)
                .await?;
            Ok(())
        }

        ApiProtocol::RestJson => {
            let method = action_config.method.as_deref().unwrap_or("DELETE");
            let path_template = action_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-JSON action requires 'path' field"))?;

            let path = path_template.replace("{resource_id}", resource_id);
            let body = action_config.body_template.as_deref();

            clients
                .http
                .rest_json_request(service, method, &path, body)
                .await?;
            Ok(())
        }

        ApiProtocol::RestXml => {
            let method = action_config.method.as_deref().unwrap_or("DELETE");
            let path_template = action_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-XML action requires 'path' field"))?;

            let path = path_template.replace("{resource_id}", resource_id);

            clients
                .http
                .rest_xml_request(service, method, &path, None)
                .await?;
            Ok(())
        }
    }
}

// =============================================================================
// Data-Driven Describe
// =============================================================================

/// Describe a single resource using JSON configuration
async fn invoke_describe(
    resource_key: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<Value> {
    let resource_def =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    let describe_config = resource_def
        .describe_config
        .as_ref()
        .ok_or_else(|| anyhow!("Describe not configured for {}", resource_key))?;

    let service = describe_config
        .service_name
        .as_deref()
        .unwrap_or(&resource_def.service);

    debug!(
        "Describing resource: {} with id: {} (service: {}, protocol: {:?})",
        resource_key, resource_id, service, describe_config.protocol
    );

    let mut result = match describe_config.protocol {
        ApiProtocol::Query => {
            let action_name = describe_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("Query describe requires 'action' field"))?;

            let id_param = describe_config.id_param.as_deref().unwrap_or("Id");
            let xml = clients
                .http
                .query_request(service, action_name, &[(id_param, resource_id)])
                .await?;
            let json = xml_to_json(&xml)?;

            // Extract from response path
            if let Some(ref path) = describe_config.response_path {
                extract_single_item(&json, path)?
            } else {
                json
            }
        }

        ApiProtocol::Json => {
            let action_name = describe_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("JSON describe requires 'action' field"))?;

            let body = if let Some(ref template) = describe_config.body_template {
                template.replace("{resource_id}", resource_id)
            } else {
                let id_param = describe_config.id_param.as_deref().unwrap_or("id");
                json!({ id_param: resource_id }).to_string()
            };

            let response = clients
                .http
                .json_request(service, action_name, &body)
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            if let Some(ref path) = describe_config.response_path {
                json.pointer(path).cloned().unwrap_or(json)
            } else {
                json
            }
        }

        ApiProtocol::RestJson => {
            let method = describe_config.method.as_deref().unwrap_or("GET");
            let path_template = describe_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-JSON describe requires 'path' field"))?;

            let path = path_template.replace("{resource_id}", resource_id);
            let response = clients
                .http
                .rest_json_request(service, method, &path, None)
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            if let Some(ref resp_path) = describe_config.response_path {
                json.pointer(resp_path).cloned().unwrap_or(json)
            } else {
                json
            }
        }

        ApiProtocol::RestXml => {
            let method = describe_config.method.as_deref().unwrap_or("GET");
            let path_template = describe_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-XML describe requires 'path' field"))?;

            let path = path_template.replace("{resource_id}", resource_id);
            let xml = clients
                .http
                .rest_xml_request(service, method, &path, None)
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(ref resp_path) = describe_config.response_path {
                json.pointer(resp_path).cloned().unwrap_or(json)
            } else {
                json
            }
        }
    };

    // Handle enrich calls (additional API calls to add more data)
    for enrich in &describe_config.enrich_calls {
        let enrich_result = execute_enrich_call(
            clients,
            service,
            resource_id,
            enrich,
            &describe_config.protocol,
        )
        .await;
        match enrich_result {
            Ok(value) => {
                if let Value::Object(ref mut map) = result {
                    map.insert(enrich.result_field.clone(), value);
                }
            }
            Err(_) => {
                if let Some(ref default) = enrich.default_value {
                    if let Value::Object(ref mut map) = result {
                        map.insert(enrich.result_field.clone(), json!(default));
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Execute an enrichment call for describe
async fn execute_enrich_call(
    clients: &AwsClients,
    service: &str,
    resource_id: &str,
    enrich: &super::protocol::EnrichCall,
    _protocol: &ApiProtocol,
) -> Result<Value> {
    // For now, support REST-XML S3 style enrich calls
    if let Some(ref path) = enrich.path {
        let path = path.replace("{resource_id}", resource_id);
        let method = enrich.method.as_deref().unwrap_or("GET");

        let xml = clients
            .http
            .rest_xml_request(service, method, &path, None)
            .await?;
        let json = xml_to_json(&xml)?;

        if let Some(ref extract) = enrich.extract_path {
            Ok(json.pointer(extract).cloned().unwrap_or(Value::Null))
        } else {
            Ok(json)
        }
    } else {
        Err(anyhow!("Enrich call requires path"))
    }
}

/// Extract a single item from a response that may be array or object
fn extract_single_item(json: &Value, path: &str) -> Result<Value> {
    let value = json
        .pointer(path)
        .ok_or_else(|| anyhow!("Response path not found: {}", path))?;

    match value {
        Value::Array(arr) => arr
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("Empty response")),
        obj @ Value::Object(_) => Ok(obj.clone()),
        _ => Ok(value.clone()),
    }
}

// =============================================================================
// Unified Action Execution
// =============================================================================

/// Execute an action on a resource (start, stop, terminate, etc.)
/// Uses JSON config to execute the action.
pub async fn execute_action(
    service: &str,
    action: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<()> {
    let (resource_key, _) = find_resource_with_action(service, action).ok_or_else(|| {
        anyhow!(
            "Action '{}' not configured for service '{}'. Add action_configs to the resource JSON.",
            action,
            service
        )
    })?;

    invoke_action(&resource_key, action, clients, resource_id).await
}

/// Find a resource that has the given action configured
fn find_resource_with_action(
    service: &str,
    action_id: &str,
) -> Option<(String, &'static super::registry::ResourceDef)> {
    use super::registry::get_registry;

    for (key, resource) in &get_registry().resources {
        if resource.service == service && resource.action_configs.contains_key(action_id) {
            return Some((key.clone(), resource));
        }
    }
    None
}

// =============================================================================
// Describe Function
// =============================================================================

/// Fetch full details for a single resource by ID
/// Uses JSON config, with special handling for S3 buckets.
pub async fn describe_resource(
    resource_key: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<Value> {
    // S3 buckets need special handling for region resolution
    if resource_key == "s3-buckets" {
        return describe_s3_bucket(clients, resource_id).await;
    }

    let resource =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    if resource.describe_config.is_none() {
        return Err(anyhow!(
            "Describe not configured for '{}'. Add describe_config to the resource JSON.",
            resource_key
        ));
    }

    invoke_describe(resource_key, clients, resource_id).await
}

/// Special handler for S3 bucket describe (needs region resolution)
async fn describe_s3_bucket(clients: &AwsClients, bucket_name: &str) -> Result<Value> {
    let mut result = json!({
        "BucketName": bucket_name,
    });

    // Get bucket location first (this determines the region for other calls)
    let bucket_region = clients
        .http
        .get_bucket_region(bucket_name)
        .await
        .unwrap_or_else(|_| "us-east-1".to_string());
    result["Region"] = json!(&bucket_region);

    // Get bucket versioning
    if let Ok(xml) = clients
        .http
        .rest_xml_request_s3_bucket("GET", bucket_name, "?versioning", None, &bucket_region)
        .await
    {
        if let Ok(json) = xml_to_json(&xml) {
            let status = json
                .pointer("/VersioningConfiguration/Status")
                .and_then(|v| v.as_str())
                .unwrap_or("Disabled");
            result["Versioning"] = json!(status);
        }
    }

    // Get bucket encryption
    if let Ok(xml) = clients
        .http
        .rest_xml_request_s3_bucket("GET", bucket_name, "?encryption", None, &bucket_region)
        .await
    {
        if let Ok(json) = xml_to_json(&xml) {
            if let Some(rules) = json.pointer("/ServerSideEncryptionConfiguration/Rule") {
                result["Encryption"] = rules.clone();
            }
        }
    } else {
        result["Encryption"] = json!("None");
    }

    Ok(result)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_api_config_returns_false_for_unknown() {
        assert!(!has_api_config("nonexistent-resource"));
    }

    #[test]
    fn test_dynamodb_tables_has_api_config() {
        assert!(has_api_config("dynamodb-tables"));
    }

    #[test]
    fn test_ec2_instances_has_api_config() {
        assert!(has_api_config("ec2-instances"));
    }

    #[test]
    fn test_lambda_functions_has_api_config() {
        assert!(has_api_config("lambda-functions"));
    }

    #[test]
    fn test_iam_users_has_api_config() {
        assert!(has_api_config("iam-users"));
    }
}
