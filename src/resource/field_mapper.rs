//! Field mapping and transformation
//!
//! This module transforms raw AWS API responses into normalized JSON
//! objects based on field mapping configuration.

use super::path_extractor::{extract_by_path, value_to_string};
use super::protocol::FieldMapping;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Apply field mappings to transform a raw API response item into normalized output
///
/// # Arguments
/// * `item` - Raw item from API response
/// * `mappings` - Map of target field name -> source field mapping
///
/// # Returns
/// Normalized JSON object with mapped fields
pub fn apply_field_mappings(item: &Value, mappings: &HashMap<String, FieldMapping>) -> Value {
    let mut result = Map::new();

    for (target_field, mapping) in mappings {
        // If source is empty or "/", use the item itself (for scalar arrays like DynamoDB table names)
        let value = if mapping.source.is_empty() || mapping.source == "/" {
            item.clone()
        } else {
            extract_by_path(item, &mapping.source)
        };

        // Apply transformation if specified
        let value = if let Some(transform) = &mapping.transform {
            apply_transform(&value, transform)
        } else {
            value
        };

        // Apply default if value is null
        let value = if value.is_null() {
            mapping
                .default
                .as_ref()
                .map(|d| Value::String(d.clone()))
                .unwrap_or(Value::String("-".to_string()))
        } else {
            // Convert non-string values to strings for consistency
            match value {
                Value::String(_) => value,
                Value::Number(n) => Value::String(n.to_string()),
                Value::Bool(b) => Value::String(if b { "Yes" } else { "No" }.to_string()),
                Value::Array(_) | Value::Object(_) => value, // Keep complex types as-is
                Value::Null => Value::String("-".to_string()),
            }
        };

        result.insert(target_field.clone(), value);
    }

    Value::Object(result)
}

/// Apply a named transformation to a value
fn apply_transform(value: &Value, transform: &str) -> Value {
    match transform {
        "tags_to_map" => transform_tags_to_map(value),
        "format_bytes" => transform_format_bytes(value),
        "format_epoch_millis" => transform_format_epoch_millis(value),
        "bool_to_yes_no" => transform_bool_to_yes_no(value),
        "array_to_csv" => transform_array_to_csv(value),
        "first_item" => transform_first_item(value),
        "private_zone_to_type" => transform_private_zone_to_type(value),
        "route53_record_value" => transform_route53_record_value(value),
        _ => value.clone(),
    }
}

/// Transform Route53 PrivateZone boolean to "Public"/"Private"
fn transform_private_zone_to_type(value: &Value) -> Value {
    match value {
        Value::Bool(b) => Value::String(if *b { "Private" } else { "Public" }.to_string()),
        Value::String(s) => {
            let is_private = s == "true" || s == "True" || s == "TRUE";
            Value::String(if is_private { "Private" } else { "Public" }.to_string())
        }
        _ => Value::String("Public".to_string()),
    }
}

/// Transform Route53 record to value string
/// Handles both ResourceRecords and AliasTarget
/// ResourceRecords: [{"Value": "192.0.2.1"}] -> "192.0.2.1"
/// AliasTarget: {"DNSName": "example.com"} -> "example.com"
fn transform_route53_record_value(value: &Value) -> Value {
    // Check for AliasTarget first
    if let Some(alias_target) = value.get("AliasTarget") {
        let dns_name = alias_target
            .get("DNSName")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        return Value::String(dns_name.to_string());
    }

    // Check for ResourceRecords
    if let Some(resource_records) = value.get("ResourceRecords") {
        if let Some(records) = resource_records.get("ResourceRecord") {
            let arr = match records {
                Value::Array(a) => a.clone(),
                obj @ Value::Object(_) => vec![obj.clone()],
                _ => return Value::String("-".to_string()),
            };

            let values: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    item.get("Value")
                        .or_else(|| item.get("value"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();

            if !values.is_empty() {
                return Value::String(values.join(", "));
            }
        }
    }

    Value::String("-".to_string())
}

/// Transform AWS tag array to a key-value map
///
/// Input: [{"key": "Name", "value": "MyInstance"}, {"Key": "Env", "Value": "prod"}]
/// Output: {"Name": "MyInstance", "Env": "prod"}
pub fn transform_tags_to_map(value: &Value) -> Value {
    let mut tags = Map::new();

    let items = match value {
        Value::Array(arr) => arr.clone(),
        Value::Object(_) => vec![value.clone()], // Single tag
        _ => return Value::Object(tags),
    };

    for tag in items {
        // AWS uses both "key"/"value" (EC2 XML) and "Key"/"Value" (other services)
        let key = tag
            .get("key")
            .or_else(|| tag.get("Key"))
            .and_then(|v| v.as_str());
        let val = tag
            .get("value")
            .or_else(|| tag.get("Value"))
            .and_then(|v| v.as_str());

        if let (Some(k), Some(v)) = (key, val) {
            tags.insert(k.to_string(), Value::String(v.to_string()));
        }
    }

    Value::Object(tags)
}

/// Format bytes into human-readable format
pub fn transform_format_bytes(value: &Value) -> Value {
    let bytes = match value {
        Value::Number(n) => n.as_u64().unwrap_or(0),
        Value::String(s) => s.parse::<u64>().unwrap_or(0),
        _ => return Value::String("-".to_string()),
    };

    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    let formatted = if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    };

    Value::String(formatted)
}

/// Format epoch milliseconds to human-readable date string
pub fn transform_format_epoch_millis(value: &Value) -> Value {
    let millis = match value {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse::<i64>().unwrap_or(0),
        _ => return Value::String("-".to_string()),
    };

    if millis <= 0 {
        return Value::String("-".to_string());
    }

    use chrono::{TimeZone, Utc};

    let formatted = Utc
        .timestamp_millis_opt(millis)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string());

    Value::String(formatted)
}

/// Transform boolean to Yes/No string
pub fn transform_bool_to_yes_no(value: &Value) -> Value {
    match value {
        Value::Bool(b) => Value::String(if *b { "Yes" } else { "No" }.to_string()),
        Value::String(s) => {
            let yes = s == "true" || s == "True" || s == "TRUE" || s == "yes" || s == "Yes";
            Value::String(if yes { "Yes" } else { "No" }.to_string())
        }
        _ => Value::String("-".to_string()),
    }
}

/// Transform array to comma-separated values
pub fn transform_array_to_csv(value: &Value) -> Value {
    match value {
        Value::Array(arr) => {
            let csv: Vec<String> = arr.iter().map(|v| value_to_string(v, "")).collect();
            Value::String(csv.join(", "))
        }
        _ => value.clone(),
    }
}

/// Extract first item from array
pub fn transform_first_item(value: &Value) -> Value {
    match value {
        Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
        _ => value.clone(),
    }
}

/// Build a normalized response with items under the specified key
pub fn build_response(items: Vec<Value>, response_key: &str, next_token: Option<String>) -> Value {
    let mut response = json!({
        response_key: items
    });

    if let Some(token) = next_token {
        response["_next_token"] = json!(token);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_field_mappings() {
        let item = json!({
            "instanceId": "i-123",
            "instanceState": {
                "name": "running"
            }
        });

        let mut mappings = HashMap::new();
        mappings.insert(
            "InstanceId".to_string(),
            FieldMapping {
                source: "/instanceId".to_string(),
                default: None,
                transform: None,
                array_item_path: None,
            },
        );
        mappings.insert(
            "State".to_string(),
            FieldMapping {
                source: "/instanceState/name".to_string(),
                default: None,
                transform: None,
                array_item_path: None,
            },
        );

        let result = apply_field_mappings(&item, &mappings);
        assert_eq!(result["InstanceId"], "i-123");
        assert_eq!(result["State"], "running");
    }

    #[test]
    fn test_apply_field_mappings_with_default() {
        let item = json!({
            "instanceId": "i-123"
        });

        let mut mappings = HashMap::new();
        mappings.insert(
            "PublicIp".to_string(),
            FieldMapping {
                source: "/publicIp".to_string(),
                default: Some("N/A".to_string()),
                transform: None,
                array_item_path: None,
            },
        );

        let result = apply_field_mappings(&item, &mappings);
        assert_eq!(result["PublicIp"], "N/A");
    }

    #[test]
    fn test_transform_tags_to_map() {
        let tags = json!([
            {"key": "Name", "value": "MyInstance"},
            {"key": "Env", "value": "prod"}
        ]);

        let result = transform_tags_to_map(&tags);
        assert_eq!(result["Name"], "MyInstance");
        assert_eq!(result["Env"], "prod");
    }

    #[test]
    fn test_transform_tags_capital_case() {
        let tags = json!([
            {"Key": "Name", "Value": "MyInstance"}
        ]);

        let result = transform_tags_to_map(&tags);
        assert_eq!(result["Name"], "MyInstance");
    }

    #[test]
    fn test_transform_format_bytes() {
        assert_eq!(transform_format_bytes(&json!(0)), json!("0 B"));
        assert_eq!(transform_format_bytes(&json!(1024)), json!("1.0 KB"));
        assert_eq!(transform_format_bytes(&json!(1048576)), json!("1.0 MB"));
        assert_eq!(transform_format_bytes(&json!(1073741824)), json!("1.0 GB"));
    }

    #[test]
    fn test_transform_bool_to_yes_no() {
        assert_eq!(transform_bool_to_yes_no(&json!(true)), json!("Yes"));
        assert_eq!(transform_bool_to_yes_no(&json!(false)), json!("No"));
        assert_eq!(transform_bool_to_yes_no(&json!("true")), json!("Yes"));
        assert_eq!(transform_bool_to_yes_no(&json!("false")), json!("No"));
    }

    #[test]
    fn test_build_response() {
        let items = vec![json!({"id": "1"}), json!({"id": "2"})];

        let response = build_response(items, "instances", Some("token123".to_string()));
        assert_eq!(response["instances"].as_array().unwrap().len(), 2);
        assert_eq!(response["_next_token"], "token123");
    }
}
