//! JSON Protocol Handler
//!
//! Handles AWS JSON-RPC protocol (used by DynamoDB, ECS, etc.)
//! - Request: POST with X-Amz-Target header, JSON body
//! - Response: JSON

use super::ProtocolHandler;
use crate::aws::client::AwsClients;
use crate::resource::path_extractor::{extract_by_path, extract_list};
use crate::resource::protocol::ApiConfig;
use anyhow::Result;
use serde_json::Value;

pub struct JsonProtocolHandler;

impl JsonProtocolHandler {
    /// Execute the API request (async implementation)
    pub async fn execute_impl(
        &self,
        clients: &AwsClients,
        service: &str,
        config: &ApiConfig,
        params: &Value,
    ) -> Result<String> {
        let action = config
            .action
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("JSON protocol requires 'action' field"))?;

        // Build request body
        let mut body = serde_json::Map::new();

        // Add static params from config
        for (key, value) in &config.static_params {
            body.insert(key.clone(), value.clone());
        }

        // Add dynamic params (skip internal params starting with '_')
        if let Value::Object(map) = params {
            for (key, value) in map {
                if !key.starts_with('_') {
                    // Apply param_mapping if defined (e.g., "log_group_name" -> "logGroupName")
                    let mapped_key = config
                        .param_mapping
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| key.clone());

                    // Unwrap single-element arrays to single values
                    // This is needed because filters are passed as arrays, but JSON protocol
                    // APIs typically expect single values (e.g., logGroupName: "name" not ["name"])
                    let unwrapped_value = match value {
                        Value::Array(arr) if arr.len() == 1 => arr[0].clone(),
                        _ => value.clone(),
                    };

                    body.insert(mapped_key, unwrapped_value);
                }
            }
        }

        let body_str = serde_json::to_string(&Value::Object(body))?;
        clients.http.json_request(service, action, &body_str).await
    }
}

impl ProtocolHandler for JsonProtocolHandler {
    fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)> {
        let json: Value = serde_json::from_str(response)?;

        // Extract items using response_root path
        let items = if let Some(root) = &config.response_root {
            extract_list(&json, root)
        } else {
            // If no response_root, try common keys
            if let Some(arr) = json.as_array() {
                arr.clone()
            } else {
                vec![json.clone()]
            }
        };

        // Extract next token if pagination is configured
        let next_token = config
            .pagination
            .as_ref()
            .and_then(|p| p.output_token.as_ref())
            .and_then(|path| {
                let token = extract_by_path(&json, path);
                token.as_str().map(|s| s.to_string())
            });

        Ok((items, next_token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_dynamodb_response() {
        let handler = JsonProtocolHandler;

        let response = r#"{
            "TableNames": ["table1", "table2", "table3"]
        }"#;

        let config = ApiConfig {
            response_root: Some("/TableNames".to_string()),
            ..Default::default()
        };

        let (items, _) = handler.parse_items(response, &config).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], "table1");
    }

    #[test]
    fn test_parse_ecs_clusters_response() {
        let response = r#"{
            "clusters": [
                {"clusterArn": "arn:aws:ecs:us-east-1:123:cluster/default", "status": "ACTIVE"},
                {"clusterArn": "arn:aws:ecs:us-east-1:123:cluster/prod", "status": "ACTIVE"}
            ]
        }"#;

        let config = ApiConfig {
            response_root: Some("/clusters".to_string()),
            ..Default::default()
        };

        let handler = JsonProtocolHandler;
        let (items, _) = handler.parse_items(response, &config).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0]["clusterArn"],
            "arn:aws:ecs:us-east-1:123:cluster/default"
        );
    }

    #[test]
    fn test_parse_with_pagination() {
        let response = r#"{
            "clusters": [{"name": "test"}],
            "nextToken": "abc123"
        }"#;

        let config = ApiConfig {
            response_root: Some("/clusters".to_string()),
            pagination: Some(crate::resource::protocol::PaginationConfig {
                output_token: Some("/nextToken".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let handler = JsonProtocolHandler;
        let (items, next_token) = handler.parse_items(response, &config).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(next_token, Some("abc123".to_string()));
    }
}
