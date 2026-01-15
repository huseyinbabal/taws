//! Path-based value extraction from JSON
//!
//! This module provides utilities to extract values from JSON using
//! path expressions (similar to JSON pointers but with array handling).

use serde_json::Value;

/// Extract a value from JSON using a path expression.
///
/// Paths use '/' as separator and support:
/// - Object field access: "/field"
/// - Nested access: "/field/subfield"
/// - Array flattening: when encountering an array, extracts from all items
///
/// # Arguments
/// * `json` - The JSON value to extract from
/// * `path` - Path expression (e.g., "/instanceState/name")
///
/// # Returns
/// The extracted value, or Value::Null if not found
pub fn extract_by_path(json: &Value, path: &str) -> Value {
    if path.is_empty() || path == "/" {
        return json.clone();
    }

    let parts: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    extract_by_parts(json, &parts)
}

fn extract_by_parts(json: &Value, parts: &[&str]) -> Value {
    if parts.is_empty() {
        return json.clone();
    }

    let part = parts[0];
    let remaining = &parts[1..];

    match json {
        Value::Object(map) => {
            if let Some(value) = map.get(part) {
                extract_by_parts(value, remaining)
            } else {
                Value::Null
            }
        }
        Value::Array(arr) => {
            // When we hit an array, extract from each item and collect results
            let results: Vec<Value> = arr
                .iter()
                .map(|item| extract_by_parts(item, parts))
                .filter(|v| !v.is_null())
                .collect();

            if results.is_empty() {
                Value::Null
            } else if results.len() == 1 {
                results.into_iter().next().unwrap()
            } else {
                Value::Array(results)
            }
        }
        _ => Value::Null,
    }
}

/// Extract a list of items from JSON using a path expression.
///
/// This is specifically designed for extracting arrays of items from
/// AWS API responses. Handles the XML-to-JSON quirk where single items
/// become objects instead of single-element arrays.
///
/// # Arguments
/// * `json` - The JSON value to extract from
/// * `path` - Path to the list (e.g., "/DescribeInstancesResponse/reservationSet/item")
///
/// # Returns
/// A vector of items (always a Vec, even if only one item)
pub fn extract_list(json: &Value, path: &str) -> Vec<Value> {
    let extracted = extract_by_path(json, path);

    match extracted {
        Value::Array(arr) => arr,
        Value::Object(_) => vec![extracted], // Single item from XML
        Value::Null => vec![],
        _ => vec![extracted], // Scalar value (unusual but handle it)
    }
}

/// Extract a string value from JSON, with optional default
#[allow(dead_code)]
pub fn extract_string(json: &Value, path: &str, default: &str) -> String {
    let value = extract_by_path(json, path);
    value_to_string(&value, default)
}

/// Convert a JSON value to a string representation
pub fn value_to_string(value: &Value, default: &str) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if *b {
                "Yes".to_string()
            } else {
                "No".to_string()
            }
        }
        Value::Null => default.to_string(),
        Value::Array(arr) => {
            // Join array elements
            arr.iter()
                .map(|v| value_to_string(v, default))
                .collect::<Vec<_>>()
                .join(", ")
        }
        Value::Object(_) => default.to_string(),
    }
}

/// Navigate through nested response structures common in AWS APIs
///
/// Many AWS XML responses have structures like:
/// - IAM: { "XXXResponse": { "XXXResult": { "Items": { "member": [...] } } } }
/// - EC2: { "XXXResponse": { "setKey": { "item": [...] } } }
/// - RDS: { "XXXResponse": { "XXXResult": { "Items": { "Item": [...] } } } }
///
/// This function handles these patterns by trying common navigation paths.
#[allow(dead_code)]
pub fn extract_aws_list(
    json: &Value,
    response_wrapper: Option<&str>,
    list_key: &str,
    item_key: &str,
) -> Vec<Value> {
    // Try direct path first
    let mut current = json.clone();

    // Navigate through response wrapper if specified
    if let Some(wrapper) = response_wrapper {
        if let Some(inner) = navigate_response_wrapper(&current, wrapper) {
            current = inner;
        }
    }

    // Navigate to list key
    if let Some(list) = current.get(list_key) {
        current = list.clone();
    } else {
        return vec![];
    }

    // Navigate to item key
    if let Some(items) = current.get(item_key) {
        match items {
            Value::Array(arr) => arr.clone(),
            obj @ Value::Object(_) => vec![obj.clone()],
            _ => vec![],
        }
    } else {
        // Maybe the list is directly an array
        match current {
            Value::Array(arr) => arr,
            obj @ Value::Object(_) => vec![obj],
            _ => vec![],
        }
    }
}

/// Navigate through AWS response wrapper (XXXResponse > XXXResult)
#[allow(dead_code)]
fn navigate_response_wrapper(json: &Value, _wrapper_hint: &str) -> Option<Value> {
    // AWS responses typically have: { "ActionResponse": { "ActionResult": { ... } } }
    // or just: { "ActionResponse": { ... } }
    if let Value::Object(map) = json {
        // Get first key (the XXXResponse)
        if let Some((_, response_value)) = map.iter().next() {
            if let Value::Object(inner_map) = response_value {
                // Check if there's a XXXResult key
                for (key, value) in inner_map {
                    if key.ends_with("Result") {
                        return Some(value.clone());
                    }
                }
                // No Result key, return the response value directly
                return Some(response_value.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_simple_path() {
        let json = json!({
            "name": "test",
            "value": 42
        });

        assert_eq!(extract_by_path(&json, "/name"), json!("test"));
        assert_eq!(extract_by_path(&json, "/value"), json!(42));
        assert_eq!(extract_by_path(&json, "/missing"), Value::Null);
    }

    #[test]
    fn test_extract_nested_path() {
        let json = json!({
            "instance": {
                "state": {
                    "name": "running"
                }
            }
        });

        assert_eq!(
            extract_by_path(&json, "/instance/state/name"),
            json!("running")
        );
    }

    #[test]
    fn test_extract_from_array() {
        let json = json!({
            "items": [
                {"id": "1", "name": "first"},
                {"id": "2", "name": "second"}
            ]
        });

        // Extract from array - should get array of values
        let result = extract_by_path(&json, "/items/name");
        assert_eq!(result, json!(["first", "second"]));
    }

    #[test]
    fn test_extract_list_array() {
        let json = json!({
            "items": [
                {"id": "1"},
                {"id": "2"}
            ]
        });

        let list = extract_list(&json, "/items");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_extract_list_single_object() {
        // XML-to-JSON often produces single object instead of array
        let json = json!({
            "items": {
                "id": "1"
            }
        });

        let list = extract_list(&json, "/items");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "1");
    }

    #[test]
    fn test_extract_string() {
        let json = json!({"name": "test", "count": 42});

        assert_eq!(extract_string(&json, "/name", "-"), "test");
        assert_eq!(extract_string(&json, "/count", "-"), "42");
        assert_eq!(extract_string(&json, "/missing", "-"), "-");
    }

    #[test]
    fn test_value_to_string_bool() {
        assert_eq!(value_to_string(&json!(true), "-"), "Yes");
        assert_eq!(value_to_string(&json!(false), "-"), "No");
    }

    #[test]
    fn test_ec2_style_response() {
        let json = json!({
            "DescribeInstancesResponse": {
                "reservationSet": {
                    "item": [
                        {
                            "instancesSet": {
                                "item": {
                                    "instanceId": "i-123",
                                    "instanceState": {
                                        "name": "running"
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        });

        let items = extract_list(
            &json,
            "/DescribeInstancesResponse/reservationSet/item/instancesSet/item",
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["instanceId"], "i-123");
    }
}
