//! Resource Fetcher - Generic fetch function driven by JSON config
//!
//! This module provides a single generic function to fetch any AWS resource.
//! All the logic is driven by the resources.json configuration.

use super::registry::get_resource;
use super::sdk_dispatch::invoke_sdk;
use crate::aws::client::AwsClients;
use anyhow::{anyhow, Result};
use serde_json::Value;

/// Filter for fetching resources (used for sub-resource filtering)
#[derive(Debug, Clone, Default)]
pub struct ResourceFilter {
    pub name: String,
    pub values: Vec<String>,
}

impl ResourceFilter {
    pub fn new(name: &str, values: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            values,
        }
    }
}

/// Result from paginated fetch including items and next page token
#[derive(Debug, Clone)]
pub struct PaginatedResult {
    pub items: Vec<Value>,
    pub next_token: Option<String>,
}

/// Fetch resources using the JSON-driven configuration
///
/// This is the SINGLE entry point for fetching any AWS resource.
/// It looks up the resource definition from JSON and uses the SDK dispatcher.
///
/// # Arguments
/// * `resource_key` - The resource key (e.g., "iam-users", "iam-roles")
/// * `clients` - AWS clients container
/// * `filters` - Optional filters for sub-resource queries
///
/// # Returns
/// A vector of JSON values representing the resources
pub async fn fetch_resources(
    resource_key: &str,
    clients: &AwsClients,
    filters: &[ResourceFilter],
) -> Result<Vec<Value>> {
    // 1. Look up resource definition from JSON
    let resource_def = get_resource(resource_key)
        .ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    // 2. Build params (merge default params with filters)
    let mut params = resource_def.sdk_method_params.clone();
    
    // Add filters to params if any
    if !filters.is_empty() {
        if let Value::Object(ref mut map) = params {
            for filter in filters {
                map.insert(filter.name.clone(), Value::Array(
                    filter.values.iter().map(|v| Value::String(v.clone())).collect()
                ));
            }
        }
    }

    // 3. Call SDK dispatcher
    let response = invoke_sdk(
        &resource_def.service,
        &resource_def.sdk_method,
        clients,
        &params,
    ).await?;

    // 4. Extract items using response_path
    let mut items = extract_items(&response, &resource_def.response_path)?;
    
    // 5. Sort items by name_field for consistent ordering
    let sort_field = &resource_def.name_field;
    items.sort_by(|a, b| {
        let a_val = a.get(sort_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let b_val = b.get(sort_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        a_val.cmp(b_val)
    });

    Ok(items)
}

/// Fetch resources with pagination support
/// 
/// Returns items for the current page and the next_token for fetching more
pub async fn fetch_resources_paginated(
    resource_key: &str,
    clients: &AwsClients,
    filters: &[ResourceFilter],
    page_token: Option<&str>,
) -> Result<PaginatedResult> {
    // 1. Look up resource definition from JSON
    let resource_def = get_resource(resource_key)
        .ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    // 2. Build params (merge default params with filters)
    let mut params = resource_def.sdk_method_params.clone();
    
    // Add filters to params if any
    if !filters.is_empty() {
        if let Value::Object(ref mut map) = params {
            for filter in filters {
                map.insert(filter.name.clone(), Value::Array(
                    filter.values.iter().map(|v| Value::String(v.clone())).collect()
                ));
            }
        }
    }
    
    // Add pagination token if provided
    if let Some(token) = page_token {
        if let Value::Object(ref mut map) = params {
            map.insert("_page_token".to_string(), Value::String(token.to_string()));
        }
    }

    // 3. Call SDK dispatcher
    let response = invoke_sdk(
        &resource_def.service,
        &resource_def.sdk_method,
        clients,
        &params,
    ).await?;

    // 4. Extract items using response_path
    let mut items = extract_items(&response, &resource_def.response_path)?;
    
    // 5. Sort items by name_field (or id_field) for consistent ordering
    let sort_field = &resource_def.name_field;
    items.sort_by(|a, b| {
        let a_val = a.get(sort_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let b_val = b.get(sort_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        a_val.cmp(b_val)
    });
    
    // 6. Extract next_token from response (if present)
    let next_token = response.get("_next_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(PaginatedResult { items, next_token })
}

/// Extract items array from response using the response_path
fn extract_items(response: &Value, path: &str) -> Result<Vec<Value>> {
    // Simple path extraction (e.g., "users", "roles")
    // For nested paths, split by '.' and traverse
    let parts: Vec<&str> = path.split('.').collect();
    
    let mut current = response.clone();
    for part in parts {
        current = current
            .get(part)
            .cloned()
            .ok_or_else(|| anyhow!("Path '{}' not found in response", path))?;
    }

    // Expect an array
    match current {
        Value::Array(arr) => Ok(arr),
        _ => Err(anyhow!("Expected array at path '{}', got {:?}", path, current)),
    }
}

/// Extract a value from a JSON object using dot notation path
/// Supports: "Field", "Field.SubField", "Field.0", "Tags.Name"
pub fn extract_json_value(item: &Value, path: &str) -> String {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = item.clone();

    for part in parts {
        current = match current {
            Value::Object(map) => {
                // Special handling for Tags.Name pattern
                if part == "Name" && map.contains_key("Tags") {
                    if let Some(Value::Object(tags)) = map.get("Tags") {
                        if let Some(Value::String(name)) = tags.get("Name") {
                            return name.clone();
                        }
                    }
                }
                map.get(part).cloned().unwrap_or(Value::Null)
            }
            Value::Array(arr) => {
                // Handle numeric index or "length"
                if part == "length" {
                    return arr.len().to_string();
                }
                if let Ok(idx) = part.parse::<usize>() {
                    arr.get(idx).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        };
    }

    // Convert final value to string
    match current {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if b {
                "Yes".to_string()
            } else {
                "No".to_string()
            }
        }
        Value::Null => "-".to_string(),
        _ => "-".to_string(),
    }
}
