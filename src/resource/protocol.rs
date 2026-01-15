//! Protocol definitions and API configuration types
//!
//! This module defines the data structures for configuring AWS API calls
//! in a data-driven way, allowing operations to be defined in JSON config
//! rather than hard-coded in Rust.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// AWS API protocol types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiProtocol {
    /// EC2/IAM style: Action=X&Version=Y as query params, XML response
    Query,
    /// JSON-RPC style with X-Amz-Target header
    Json,
    /// REST with JSON body (Lambda, EKS)
    RestJson,
    /// REST with XML body (S3, Route53)
    RestXml,
}

impl Default for ApiProtocol {
    fn default() -> Self {
        ApiProtocol::Query
    }
}

/// Pagination configuration for API calls
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PaginationConfig {
    /// Parameter name for input token (e.g., "NextToken", "Marker")
    #[serde(default)]
    pub input_token: Option<String>,
    /// Path to extract output token from response
    #[serde(default)]
    pub output_token: Option<String>,
    /// Parameter name for max results
    #[serde(default)]
    pub max_results_param: Option<String>,
    /// Default max results value
    #[serde(default)]
    pub max_results: Option<u32>,
}

/// Configuration for a single API operation
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ApiConfig {
    /// Protocol to use for this API call
    #[serde(default)]
    pub protocol: ApiProtocol,

    /// Override the service name for API calls (e.g., "events" instead of "eventbridge")
    /// If not specified, uses the resource's service field
    #[serde(default)]
    pub service_name: Option<String>,

    /// Action name (for Query/JSON protocols, e.g., "DescribeInstances")
    #[serde(default)]
    pub action: Option<String>,

    /// HTTP method (for REST protocols: GET, POST, DELETE, etc.)
    #[serde(default)]
    pub method: Option<String>,

    /// URL path template (for REST protocols, e.g., "/2015-03-31/functions")
    /// Supports placeholders like {function_name}
    #[serde(default)]
    pub path: Option<String>,

    /// Path to extract items from the response (JSON pointer format)
    /// For XML: after xml_to_json conversion
    /// e.g., "/DescribeInstancesResponse/reservationSet/item/instancesSet/item"
    #[serde(default)]
    pub response_root: Option<String>,

    /// For nested responses, path to get to the items array within response_root
    /// Used when items are wrapped in another object
    #[serde(default)]
    pub items_path: Option<String>,

    /// Static parameters to always include in the request
    #[serde(default)]
    pub static_params: HashMap<String, Value>,

    /// Map filter parameter names to API parameter names
    /// e.g., {"log_group_name": "logGroupName"}
    #[serde(default)]
    pub param_mapping: HashMap<String, String>,

    /// Pagination configuration
    #[serde(default)]
    pub pagination: Option<PaginationConfig>,

    /// Pre-request hooks (e.g., "resolve_s3_bucket_region")
    #[serde(default)]
    pub pre_hooks: Vec<String>,

    /// Whether this is a composite operation requiring multiple API calls
    #[serde(default)]
    pub composite: bool,

    /// For composite operations, the sequence of operations
    #[serde(default)]
    pub operations: Vec<CompositeOperation>,
}

/// A single operation in a composite API call
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CompositeOperation {
    /// Action name for this step
    pub action: String,
    /// Store result in this variable name
    #[serde(default)]
    pub store_as: Option<String>,
    /// Input parameters (can reference previous results with $variable)
    #[serde(default)]
    pub input: HashMap<String, Value>,
}

/// Field mapping from raw API response to normalized output
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FieldMapping {
    /// Source path in the API response (JSON pointer format)
    /// e.g., "/instanceId" or "/instanceState/name"
    pub source: String,

    /// Default value if source is null/missing
    #[serde(default)]
    pub default: Option<String>,

    /// Optional transformation to apply
    /// Supported: "tags_to_map", "format_bytes", "format_epoch_millis", "bool_to_yes_no"
    #[serde(default)]
    pub transform: Option<String>,

    /// For array fields, path within each array item
    #[serde(default)]
    pub array_item_path: Option<String>,
}

impl Default for FieldMapping {
    fn default() -> Self {
        Self {
            source: String::new(),
            default: Some("-".to_string()),
            transform: None,
            array_item_path: None,
        }
    }
}

/// Result from parsing an API response
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    /// The parsed items
    pub items: Vec<Value>,
    /// Next page token if pagination is available
    pub next_token: Option<String>,
}

/// Configuration for an action (write operation like start/stop/delete)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActionConfig {
    /// The action identifier (e.g., "start_instance", "delete_bucket")
    pub action_id: String,

    /// Protocol to use for this action
    pub protocol: ApiProtocol,

    /// Service name override (if different from resource's service)
    #[serde(default)]
    pub service_name: Option<String>,

    /// API action name (for Query/JSON protocols, e.g., "StartInstances")
    #[serde(default)]
    pub action: Option<String>,

    /// HTTP method (for REST protocols: POST, DELETE, etc.)
    #[serde(default)]
    pub method: Option<String>,

    /// URL path template (for REST protocols)
    /// Supports placeholders like {resource_id}
    #[serde(default)]
    pub path: Option<String>,

    /// Parameter name for the resource ID in the request
    /// e.g., "InstanceId.1" for EC2, "DBInstanceIdentifier" for RDS
    #[serde(default)]
    pub id_param: Option<String>,

    /// Static parameters to always include
    #[serde(default)]
    pub static_params: HashMap<String, Value>,

    /// For JSON body requests, the body template
    /// Supports {resource_id} placeholder
    #[serde(default)]
    pub body_template: Option<String>,

    /// Special handling needed (e.g., "parse_arn_for_cluster" for ECS)
    #[serde(default)]
    pub special_handling: Option<String>,
}

/// Configuration for describe operation (single resource details)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DescribeConfig {
    /// Protocol to use
    pub protocol: ApiProtocol,

    /// Service name override
    #[serde(default)]
    pub service_name: Option<String>,

    /// API action name (e.g., "DescribeInstances", "GetFunction")
    #[serde(default)]
    pub action: Option<String>,

    /// HTTP method (for REST)
    #[serde(default)]
    pub method: Option<String>,

    /// URL path template (for REST)
    #[serde(default)]
    pub path: Option<String>,

    /// Parameter name for the resource ID
    #[serde(default)]
    pub id_param: Option<String>,

    /// Path to extract the single resource from response
    #[serde(default)]
    pub response_path: Option<String>,

    /// For JSON body requests
    #[serde(default)]
    pub body_template: Option<String>,

    /// Additional API calls to enrich the response (e.g., S3 bucket versioning/encryption)
    #[serde(default)]
    pub enrich_calls: Vec<EnrichCall>,
}

/// Additional API call to enrich describe response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnrichCall {
    /// API action or path
    pub action: Option<String>,
    pub path: Option<String>,
    pub method: Option<String>,
    /// Field name to store result
    pub result_field: String,
    /// Path to extract value from response
    pub extract_path: Option<String>,
    /// Default value if call fails
    #[serde(default)]
    pub default_value: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_protocol_deserialize() {
        let json = r#""query""#;
        let protocol: ApiProtocol = serde_json::from_str(json).unwrap();
        assert_eq!(protocol, ApiProtocol::Query);

        let json = r#""json""#;
        let protocol: ApiProtocol = serde_json::from_str(json).unwrap();
        assert_eq!(protocol, ApiProtocol::Json);

        let json = r#""rest-json""#;
        let protocol: ApiProtocol = serde_json::from_str(json).unwrap();
        assert_eq!(protocol, ApiProtocol::RestJson);

        let json = r#""rest-xml""#;
        let protocol: ApiProtocol = serde_json::from_str(json).unwrap();
        assert_eq!(protocol, ApiProtocol::RestXml);
    }

    #[test]
    fn test_field_mapping_deserialize() {
        let json = r#"{"source": "/instanceId"}"#;
        let mapping: FieldMapping = serde_json::from_str(json).unwrap();
        assert_eq!(mapping.source, "/instanceId");
        assert_eq!(mapping.default, None);

        let json = r#"{"source": "/ipAddress", "default": "-"}"#;
        let mapping: FieldMapping = serde_json::from_str(json).unwrap();
        assert_eq!(mapping.source, "/ipAddress");
        assert_eq!(mapping.default, Some("-".to_string()));
    }

    #[test]
    fn test_api_config_deserialize() {
        let json = r#"{
            "protocol": "query",
            "action": "DescribeInstances",
            "response_root": "/DescribeInstancesResponse/reservationSet"
        }"#;
        let config: ApiConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.protocol, ApiProtocol::Query);
        assert_eq!(config.action, Some("DescribeInstances".to_string()));
    }
}
