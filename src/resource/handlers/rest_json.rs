//! REST-JSON Protocol Handler
//!
//! Handles AWS REST-JSON protocol (used by Lambda, EKS, API Gateway, etc.)
//! - Request: HTTP method with JSON body, path parameters
//! - Response: JSON

use super::ProtocolHandler;
use crate::aws::client::AwsClients;
use crate::resource::path_extractor::{extract_by_path, extract_list};
use crate::resource::protocol::ApiConfig;
use anyhow::Result;
use serde_json::Value;

pub struct RestJsonProtocolHandler;

impl RestJsonProtocolHandler {
    /// Execute the API request (async implementation)
    pub async fn execute_impl(
        &self,
        clients: &AwsClients,
        service: &str,
        config: &ApiConfig,
        params: &Value,
    ) -> Result<String> {
        let method = config.method.as_deref().unwrap_or("GET");
        let path_template = config
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("REST-JSON protocol requires 'path' field"))?;

        // Replace path parameters (e.g., {function_name})
        let mut path = path_template.clone();
        if let Value::Object(map) = params {
            for (key, value) in map {
                if let Some(s) = value.as_str() {
                    path = path.replace(&format!("{{{}}}", key), s);
                }
            }
        }

        // Add query parameters for GET requests
        if method == "GET" {
            let mut query_parts: Vec<String> = Vec::new();

            // Add pagination token if present
            if let Some(token) = params.get("_page_token").and_then(|v| v.as_str()) {
                if let Some(pagination) = &config.pagination {
                    if let Some(input_token) = &pagination.input_token {
                        query_parts.push(format!("{}={}", input_token, urlencoding::encode(token)));
                    }
                }
            }

            // Add max results if configured
            if let Some(pagination) = &config.pagination {
                if let Some(max_param) = &pagination.max_results_param {
                    let max_value = pagination.max_results.unwrap_or(50);
                    query_parts.push(format!("{}={}", max_param, max_value));
                }
            }

            if !query_parts.is_empty() {
                if path.contains('?') {
                    path = format!("{}&{}", path, query_parts.join("&"));
                } else {
                    path = format!("{}?{}", path, query_parts.join("&"));
                }
            }
        }

        // Build request body for non-GET requests
        let body = if method != "GET" {
            let mut body = serde_json::Map::new();
            for (key, value) in &config.static_params {
                body.insert(key.clone(), value.clone());
            }
            if let Value::Object(map) = params {
                for (key, value) in map {
                    if !key.starts_with('_') {
                        // Unwrap single-element arrays to single values
                        let unwrapped_value = match value {
                            Value::Array(arr) if arr.len() == 1 => arr[0].clone(),
                            _ => value.clone(),
                        };
                        body.insert(key.clone(), unwrapped_value);
                    }
                }
            }
            if body.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&Value::Object(body))?)
            }
        } else {
            None
        };

        clients
            .http
            .rest_json_request(service, method, &path, body.as_deref())
            .await
    }
}

impl ProtocolHandler for RestJsonProtocolHandler {
    fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)> {
        let json: Value = serde_json::from_str(response)?;

        // Extract items using response_root path
        let items = if let Some(root) = &config.response_root {
            extract_list(&json, root)
        } else if let Some(arr) = json.as_array() {
            arr.clone()
        } else {
            vec![json.clone()]
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
    fn test_parse_lambda_functions_response() {
        let response = r#"{
            "Functions": [
                {"FunctionName": "func1", "Runtime": "nodejs18.x"},
                {"FunctionName": "func2", "Runtime": "python3.9"}
            ],
            "NextMarker": "next123"
        }"#;

        let config = ApiConfig {
            response_root: Some("/Functions".to_string()),
            pagination: Some(crate::resource::protocol::PaginationConfig {
                output_token: Some("/NextMarker".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let handler = RestJsonProtocolHandler;
        let (items, next_token) = handler.parse_items(response, &config).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["FunctionName"], "func1");
        assert_eq!(next_token, Some("next123".to_string()));
    }

    #[test]
    fn test_parse_eks_clusters_response() {
        let response = r#"{
            "clusters": ["cluster1", "cluster2"]
        }"#;

        let config = ApiConfig {
            response_root: Some("/clusters".to_string()),
            ..Default::default()
        };

        let handler = RestJsonProtocolHandler;
        let (items, _) = handler.parse_items(response, &config).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "cluster1");
    }
}
