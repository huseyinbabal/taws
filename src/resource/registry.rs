//! Resource Registry - Load resource definitions from JSON
//!
//! This module loads all AWS resource definitions from embedded JSON files
//! and provides lookup functions for the rest of the application.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::protocol::{ActionConfig, ApiConfig, DescribeConfig, FieldMapping};

/// Embedded resource JSON files (compiled into the binary)
const RESOURCE_FILES: &[&str] = &[
    include_str!("../resources/acm.json"),
    include_str!("../resources/apigateway.json"),
    include_str!("../resources/athena.json"),
    include_str!("../resources/autoscaling.json"),
    include_str!("../resources/cloudformation.json"),
    include_str!("../resources/cloudfront.json"),
    include_str!("../resources/cloudtrail.json"),
    include_str!("../resources/cloudwatch.json"),
    include_str!("../resources/codebuild.json"),
    include_str!("../resources/codepipeline.json"),
    include_str!("../resources/cognito.json"),
    include_str!("../resources/common.json"),
    include_str!("../resources/dynamodb.json"),
    include_str!("../resources/ec2.json"),
    include_str!("../resources/ecr.json"),
    include_str!("../resources/ecs.json"),
    include_str!("../resources/eks.json"),
    include_str!("../resources/elasticache.json"),
    include_str!("../resources/elbv2.json"),
    include_str!("../resources/eventbridge.json"),
    include_str!("../resources/iam.json"),
    include_str!("../resources/kms.json"),
    include_str!("../resources/lambda.json"),
    include_str!("../resources/rds.json"),
    include_str!("../resources/route53.json"),
    include_str!("../resources/s3.json"),
    include_str!("../resources/secretsmanager.json"),
    include_str!("../resources/sns.json"),
    include_str!("../resources/sqs.json"),
    include_str!("../resources/ssm.json"),
    include_str!("../resources/sts.json"),
    include_str!("../resources/vpc.json"),
];

/// Color definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ColorDef {
    pub value: String,
    pub color: [u8; 3],
}

/// Column definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ColumnDef {
    pub header: String,
    pub json_path: String,
    pub width: u16,
    #[serde(default)]
    pub color_map: Option<String>,
}

/// Sub-resource definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct SubResourceDef {
    pub resource_key: String,
    pub display_name: String,
    pub shortcut: String,
    pub parent_id_field: String,
    pub filter_param: String,
    /// Filter type: "scalar" (default) for single-value params (IAM, ELBv2, RDS),
    /// "ec2_filter" for EC2-style Filter.N.Name/Value params (VPC subnets, security groups)
    #[serde(default = "default_filter_type")]
    pub filter_type: String,
}

fn default_filter_type() -> String {
    "scalar".to_string()
}

/// Confirmation config for actions
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConfirmConfig {
    /// Message to show in confirmation dialog
    #[serde(default)]
    pub message: Option<String>,
    /// If true, default selection is Yes; if false, default is No
    #[serde(default)]
    pub default_yes: bool,
    /// If true, action is destructive (shown in red)
    #[serde(default)]
    pub destructive: bool,
}

/// Filters configuration for resources that support AWS API filtering
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FiltersConfig {
    /// Whether this resource supports filtering via AWS API
    #[serde(default)]
    pub enabled: bool,
    /// Hint text showing available filter keys (e.g., "owner, architecture, state")
    #[serde(default)]
    pub hint: Option<String>,
}

/// Action definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ActionDef {
    /// Key identifier for the action (kept for JSON compatibility)
    #[allow(dead_code)]
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub shortcut: Option<String>,
    pub sdk_method: String,
    /// Parameter name for the resource ID (kept for potential future use)
    #[serde(default)]
    #[allow(dead_code)]
    pub id_param: Option<String>,
    /// Legacy field - use `confirm` instead
    #[serde(default)]
    pub needs_confirm: bool,
    /// Confirmation configuration
    #[serde(default)]
    pub confirm: Option<ConfirmConfig>,
    /// If true, display the action result in the JSON viewer instead of just executing
    #[serde(default)]
    pub show_result: bool,
}

impl ActionDef {
    /// Check if this action requires confirmation
    pub fn requires_confirm(&self) -> bool {
        self.confirm.is_some() || self.needs_confirm
    }

    /// Get the confirmation config (with defaults)
    pub fn get_confirm_config(&self) -> Option<ConfirmConfig> {
        if let Some(ref config) = self.confirm {
            Some(config.clone())
        } else if self.needs_confirm {
            Some(ConfirmConfig {
                message: Some(self.display_name.clone()),
                default_yes: false,
                destructive: false,
            })
        } else {
            None
        }
    }
}

/// Resource definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceDef {
    pub display_name: String,
    pub service: String,
    /// Legacy: SDK method name (used by old sdk_dispatch.rs)
    /// New resources should use api_config instead
    pub sdk_method: String,
    #[serde(default)]
    pub sdk_method_params: Value,
    pub response_path: String,
    pub id_field: String,
    pub name_field: String,
    #[serde(default)]
    pub is_global: bool,
    pub columns: Vec<ColumnDef>,
    #[serde(default)]
    pub sub_resources: Vec<SubResourceDef>,
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    /// SDK method to call when fetching details for a single resource
    #[serde(default)]
    pub detail_sdk_method: Option<String>,
    /// Parameters for detail_sdk_method (maps param name -> field from resource)
    #[serde(default)]
    pub detail_sdk_method_params: Value,

    // === NEW DATA-DRIVEN FIELDS ===
    /// API configuration for data-driven dispatch (list operations)
    /// If present, this takes precedence over sdk_method for fetching
    #[serde(default)]
    pub api_config: Option<ApiConfig>,

    /// Field mappings from raw API response to normalized output
    /// If present, these are used to transform API responses
    #[serde(default)]
    pub field_mappings: HashMap<String, FieldMapping>,

    /// Data-driven action configurations
    /// Maps action_id (e.g., "start_instance") to its API config
    #[serde(default)]
    pub action_configs: HashMap<String, ActionConfig>,

    /// Data-driven describe configuration
    /// For fetching single resource details
    #[serde(default)]
    pub describe_config: Option<DescribeConfig>,

    /// Filters configuration
    /// If present and enabled, the resource supports AWS API filtering (Filters: key=value)
    #[serde(default)]
    pub filters_config: Option<FiltersConfig>,

    /// If true, this resource requires a parent context and cannot be accessed directly
    /// Used for sub-resources like Log Streams that need a Log Group
    #[serde(default)]
    pub requires_parent: bool,
}

impl ResourceDef {
    /// Check if this resource has API config for list operations
    pub fn has_api_config(&self) -> bool {
        self.api_config.is_some() && !self.field_mappings.is_empty()
    }

    /// Check if this resource supports filtering via AWS API
    pub fn supports_filters(&self) -> bool {
        self.filters_config
            .as_ref()
            .map(|fc| fc.enabled)
            .unwrap_or(false)
    }

    /// Get the filter hint for this resource
    pub fn filters_hint(&self) -> Option<&str> {
        self.filters_config
            .as_ref()
            .and_then(|fc| fc.hint.as_deref())
    }
}

/// Root structure of resources/*.json
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceConfig {
    #[serde(default)]
    pub color_maps: HashMap<String, Vec<ColorDef>>,
    #[serde(default)]
    pub resources: HashMap<String, ResourceDef>,
}

/// Global registry loaded from JSON
static REGISTRY: OnceLock<ResourceConfig> = OnceLock::new();

/// Get the resource registry (loads from embedded JSON on first access)
pub fn get_registry() -> &'static ResourceConfig {
    REGISTRY.get_or_init(|| {
        let mut final_config = ResourceConfig {
            color_maps: HashMap::new(),
            resources: HashMap::new(),
        };

        for content in RESOURCE_FILES {
            let partial: ResourceConfig = serde_json::from_str(content)
                .unwrap_or_else(|e| panic!("Failed to parse embedded resource JSON: {}", e));
            final_config.color_maps.extend(partial.color_maps);
            final_config.resources.extend(partial.resources);
        }

        final_config
    })
}

/// Get a resource definition by key
pub fn get_resource(key: &str) -> Option<&'static ResourceDef> {
    get_registry().resources.get(key)
}

/// Get all resource keys (for autocomplete)
/// Excludes resources that require a parent context (like log-streams, ecs-tasks, etc.)
pub fn get_all_resource_keys() -> Vec<&'static str> {
    get_registry()
        .resources
        .iter()
        .filter(|(_, def)| !def.requires_parent)
        .map(|(key, _)| key.as_str())
        .collect()
}

/// Get a color map by name
pub fn get_color_map(name: &str) -> Option<&'static Vec<ColorDef>> {
    get_registry().color_maps.get(name)
}

/// Get color for a value based on color map name
pub fn get_color_for_value(color_map_name: &str, value: &str) -> Option<[u8; 3]> {
    get_color_map(color_map_name)?
        .iter()
        .find(|c| c.value == value)
        .map(|c| c.color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_loads_successfully() {
        let registry = get_registry();
        assert!(
            !registry.resources.is_empty(),
            "Registry should have resources"
        );
    }

    #[test]
    fn test_ec2_instances_resource_exists() {
        let resource = get_resource("ec2-instances");
        assert!(resource.is_some(), "EC2 instances resource should exist");

        let resource = resource.unwrap();
        assert_eq!(resource.display_name, "EC2 Instances");
        assert_eq!(resource.service, "ec2");
        assert_eq!(resource.sdk_method, "describe_instances");
        assert!(
            !resource.columns.is_empty(),
            "EC2 instances should have columns"
        );
    }

    #[test]
    fn test_iam_users_resource_exists() {
        let resource = get_resource("iam-users");
        assert!(resource.is_some(), "IAM users resource should exist");

        let resource = resource.unwrap();
        assert_eq!(resource.service, "iam");
        assert!(resource.is_global, "IAM should be a global service");
    }

    #[test]
    fn test_iam_users_has_sub_resources() {
        let resource = get_resource("iam-users").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "IAM users should have sub-resources"
        );

        let policy_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "iam-user-policies");
        assert!(
            policy_sub.is_some(),
            "IAM users should have policies sub-resource"
        );
    }

    #[test]
    fn test_ec2_instances_has_actions() {
        let resource = get_resource("ec2-instances").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "EC2 instances should have actions"
        );

        let start_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "start_instance");
        assert!(start_action.is_some(), "EC2 should have start action");

        let reboot_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "reboot_instance");
        assert!(reboot_action.is_some(), "EC2 should have reboot action");

        let terminate_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "terminate_instance");
        assert!(
            terminate_action.is_some(),
            "EC2 should have terminate action"
        );
        assert!(
            terminate_action.unwrap().requires_confirm(),
            "Terminate should require confirmation"
        );
    }

    #[test]
    fn test_get_all_resource_keys() {
        let keys = get_all_resource_keys();
        assert!(keys.len() >= 30, "Should have at least 30 resource types");
        assert!(
            keys.contains(&"ec2-instances"),
            "Should contain ec2-instances"
        );
        assert!(
            keys.contains(&"lambda-functions"),
            "Should contain lambda-functions"
        );
        assert!(keys.contains(&"s3-buckets"), "Should contain s3-buckets");
    }

    #[test]
    fn test_common_color_maps_exist() {
        let state_map = get_color_map("state");
        assert!(state_map.is_some(), "State color map should exist");

        let bool_map = get_color_map("bool");
        assert!(bool_map.is_some(), "Bool color map should exist");
    }

    #[test]
    fn test_get_color_for_running_state() {
        let color = get_color_for_value("state", "running");
        assert!(color.is_some(), "Should have color for 'running' state");
        // Green color
        assert_eq!(color.unwrap(), [0, 255, 0]);
    }

    #[test]
    fn test_rds_has_sub_resources() {
        let resource = get_resource("rds-instances").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "RDS should have sub-resources"
        );

        let snapshot_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "rds-snapshots");
        assert!(
            snapshot_sub.is_some(),
            "RDS should have snapshots sub-resource"
        );
    }

    #[test]
    fn test_ecs_has_sub_resources() {
        let resource = get_resource("ecs-clusters").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "ECS clusters should have sub-resources"
        );

        let services_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "ecs-services");
        assert!(
            services_sub.is_some(),
            "ECS should have services sub-resource"
        );

        let tasks_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "ecs-tasks");
        assert!(tasks_sub.is_some(), "ECS should have tasks sub-resource");
    }

    #[test]
    fn test_lambda_has_actions() {
        let resource = get_resource("lambda-functions").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "Lambda functions should have actions"
        );

        let invoke_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "invoke_function");
        assert!(invoke_action.is_some(), "Lambda should have invoke action");
    }

    #[test]
    fn test_all_resources_have_required_fields() {
        let registry = get_registry();
        for (key, resource) in &registry.resources {
            assert!(
                !resource.display_name.is_empty(),
                "Resource {} should have display_name",
                key
            );
            assert!(
                !resource.service.is_empty(),
                "Resource {} should have service",
                key
            );
            assert!(
                !resource.sdk_method.is_empty(),
                "Resource {} should have sdk_method",
                key
            );
            assert!(
                !resource.id_field.is_empty(),
                "Resource {} should have id_field",
                key
            );
            assert!(
                !resource.name_field.is_empty(),
                "Resource {} should have name_field",
                key
            );
        }
    }

    #[test]
    fn test_elbv2_load_balancers_resource_exists() {
        let resource = get_resource("elbv2-load-balancers");
        assert!(
            resource.is_some(),
            "ELBv2 load balancers resource should exist"
        );

        let resource = resource.unwrap();
        assert_eq!(resource.display_name, "Load Balancers");
        assert_eq!(resource.service, "elbv2");
        assert_eq!(resource.sdk_method, "describe_load_balancers");
    }

    #[test]
    fn test_elbv2_has_sub_resources() {
        let resource = get_resource("elbv2-load-balancers").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "ELBv2 load balancers should have sub-resources"
        );

        let listeners_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-listeners");
        assert!(
            listeners_sub.is_some(),
            "ELBv2 should have listeners sub-resource"
        );

        let target_groups_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-target-groups");
        assert!(
            target_groups_sub.is_some(),
            "ELBv2 should have target groups sub-resource"
        );
    }

    #[test]
    fn test_elbv2_listeners_has_rules_sub_resource() {
        let resource = get_resource("elbv2-listeners").unwrap();

        let rules_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-rules");
        assert!(
            rules_sub.is_some(),
            "ELBv2 listeners should have rules sub-resource"
        );
    }

    #[test]
    fn test_elbv2_target_groups_has_targets_sub_resource() {
        let resource = get_resource("elbv2-target-groups").unwrap();

        let targets_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-targets");
        assert!(
            targets_sub.is_some(),
            "ELBv2 target groups should have targets sub-resource"
        );
    }

    #[test]
    fn test_elbv2_health_color_map_exists() {
        let health_map = get_color_map("health");
        assert!(health_map.is_some(), "Health color map should exist");

        let color = get_color_for_value("health", "healthy");
        assert!(color.is_some(), "Should have color for 'healthy' state");
        assert_eq!(color.unwrap(), [0, 255, 0]); // Green color
    }

    #[test]
    fn test_secretsmanager_has_view_value_action() {
        let resource = get_resource("secretsmanager-secrets").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "Secrets Manager should have actions"
        );

        let view_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "get_secret_value");
        assert!(
            view_action.is_some(),
            "Secrets Manager should have get_secret_value action"
        );

        let view_action = view_action.unwrap();
        assert!(
            view_action.show_result,
            "get_secret_value action should have show_result=true"
        );
        assert_eq!(
            view_action.shortcut.as_deref(),
            Some("x"),
            "get_secret_value should use 'x' shortcut"
        );
    }
}
