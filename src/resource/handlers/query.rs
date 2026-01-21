//! Query Protocol Handler
//!
//! Handles AWS Query protocol (used by EC2, IAM, RDS, etc.)
//! - Request: GET with Action=X&Version=Y as query params
//! - Response: XML

use super::ProtocolHandler;
use crate::aws::client::AwsClients;
use crate::aws::http::xml_to_json;
use crate::resource::path_extractor::extract_list;
use crate::resource::protocol::ApiConfig;
use anyhow::Result;
use serde_json::Value;

pub struct QueryProtocolHandler;

impl QueryProtocolHandler {
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
            .ok_or_else(|| anyhow::anyhow!("Query protocol requires 'action' field"))?;

        // Build query parameters from params Value
        let mut query_params: Vec<(String, String)> = Vec::new();

        // Add static params from config
        for (key, value) in &config.static_params {
            if let Some(s) = value.as_str() {
                query_params.push((key.clone(), s.to_string()));
            }
        }

        // Track filter index for tag filters
        let mut filter_index = 1;

        // Add dynamic params
        if let Value::Object(map) = params {
            for (key, value) in map {
                // Skip internal params
                if key.starts_with('_') {
                    continue;
                }

                // Handle tag filters specially (format: "tag:KeyName" -> Filter.N.Name=tag:KeyName, Filter.N.Value.1=value)
                if key.starts_with("tag:") {
                    if let Value::Array(arr) = value {
                        // Add Filter.N.Name=tag:KeyName
                        query_params.push((format!("Filter.{}.Name", filter_index), key.clone()));
                        // Add Filter.N.Value.M for each value
                        for (i, item) in arr.iter().enumerate() {
                            if let Some(s) = item.as_str() {
                                query_params.push((
                                    format!("Filter.{}.Value.{}", filter_index, i + 1),
                                    s.to_string(),
                                ));
                            }
                        }
                        filter_index += 1;
                    }
                    continue;
                }

                // Apply param_mapping if defined (e.g., "user_name" -> "UserName")
                let mapped_key = config
                    .param_mapping
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| key.clone());

                match value {
                    Value::String(s) => {
                        query_params.push((mapped_key, s.clone()));
                    }
                    Value::Array(arr) => {
                        // Handle array params (e.g., InstanceId.1, InstanceId.2)
                        for (i, item) in arr.iter().enumerate() {
                            if let Some(s) = item.as_str() {
                                query_params
                                    .push((format!("{}.{}", mapped_key, i + 1), s.to_string()));
                            }
                        }
                    }
                    Value::Number(n) => {
                        query_params.push((mapped_key, n.to_string()));
                    }
                    Value::Bool(b) => {
                        query_params.push((mapped_key, b.to_string()));
                    }
                    _ => {}
                }
            }
        }

        // Convert to slice of tuples for the HTTP client
        let params_refs: Vec<(&str, &str)> = query_params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        clients
            .http
            .query_request(service, action, &params_refs)
            .await
    }
}

impl ProtocolHandler for QueryProtocolHandler {
    fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)> {
        // Convert XML to JSON
        let json = xml_to_json(response)?;

        // Extract items using response_root path
        let items = if let Some(root) = &config.response_root {
            extract_list(&json, root)
        } else {
            vec![]
        };

        // Extract next token if pagination is configured
        let next_token = config
            .pagination
            .as_ref()
            .and_then(|p| p.output_token.as_ref())
            .and_then(|path| {
                let token = crate::resource::path_extractor::extract_by_path(&json, path);
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
    fn test_parse_ec2_response() {
        let handler = QueryProtocolHandler;

        // Simulated EC2 DescribeInstances XML-to-JSON response
        let response = r#"
        {
            "DescribeInstancesResponse": {
                "reservationSet": {
                    "item": {
                        "instancesSet": {
                            "item": {
                                "instanceId": "i-12345",
                                "instanceState": {
                                    "name": "running"
                                }
                            }
                        }
                    }
                }
            }
        }
        "#;

        // We need to test with actual XML, but for unit test we can test JSON parsing
        let json: Value = serde_json::from_str(response).unwrap();
        let items = extract_list(
            &json,
            "/DescribeInstancesResponse/reservationSet/item/instancesSet/item",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["instanceId"], "i-12345");
    }
}
