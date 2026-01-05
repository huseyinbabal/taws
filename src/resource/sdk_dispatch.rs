//! SDK Dispatcher - AWS API calls using lightweight HTTP client
//!
//! This module handles all AWS API calls using direct HTTP with SigV4 signing.
//! Supports 30 core AWS services without heavy SDK dependencies.

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
    params.get(key)
        .and_then(|v| {
            v.as_str().map(|s| s.to_string())
             .or_else(|| v.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()).map(|s| s.to_string()))
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
    use std::time::{Duration, UNIX_EPOCH};
    
    let duration = Duration::from_millis(millis as u64);
    let datetime = UNIX_EPOCH + duration;
    
    // Convert to a simple date/time string
    if let Ok(elapsed) = datetime.duration_since(UNIX_EPOCH) {
        let secs = elapsed.as_secs();
        let days = secs / 86400;
        let years = 1970 + days / 365;
        let remaining_days = days % 365;
        let months = remaining_days / 30;
        let day = remaining_days % 30 + 1;
        let hours = (secs % 86400) / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;
        
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", 
            years, months + 1, day, hours, minutes, seconds)
    } else {
        "-".to_string()
    }
}

/// Format epoch milliseconds to human-readable date string (public for log tail UI)
pub fn format_log_timestamp(millis: i64) -> String {
    format_epoch_millis(millis)
}

/// Parse XML list response from Query protocol APIs
#[allow(dead_code)]
fn parse_query_list(xml: &str, list_key: &str, item_key: &str) -> Result<Vec<Value>> {
    let json = xml_to_json(xml)?;
    
    // Navigate through the response structure
    // Typical structure: { "XXXResponse": { "XXXResult": { "Items": { "member": [...] } } } }
    let items = json.as_object()
        .and_then(|o| o.values().next())
        .and_then(|v| v.as_object())
        .and_then(|o| o.values().next())
        .and_then(|v| v.get(list_key))
        .and_then(|v| v.get(item_key))
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    
    match items {
        Value::Array(arr) => Ok(arr),
        Value::Object(_) => Ok(vec![items]), // Single item
        _ => Ok(vec![]),
    }
}

// =============================================================================
// Action Functions (write operations)
// =============================================================================

/// Execute an action on a resource (start, stop, terminate, etc.)
pub async fn execute_action(
    service: &str,
    action: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<()> {
    match (service, action) {
        // EC2 Instance Actions
        ("ec2", "start_instance") => {
            clients.http.query_request("ec2", "StartInstances", &[
                ("InstanceId.1", resource_id)
            ]).await?;
            Ok(())
        }
        ("ec2", "stop_instance") => {
            clients.http.query_request("ec2", "StopInstances", &[
                ("InstanceId.1", resource_id)
            ]).await?;
            Ok(())
        }
        ("ec2", "terminate_instance") => {
            clients.http.query_request("ec2", "TerminateInstances", &[
                ("InstanceId.1", resource_id)
            ]).await?;
            Ok(())
        }

        // Lambda Actions
        ("lambda", "invoke_function") => {
            clients.http.rest_json_request(
                "lambda",
                "POST",
                &format!("/2015-03-31/functions/{}/invocations", resource_id),
                Some("{}")
            ).await?;
            Ok(())
        }
        ("lambda", "delete_function") => {
            clients.http.rest_json_request(
                "lambda",
                "DELETE",
                &format!("/2015-03-31/functions/{}", resource_id),
                None
            ).await?;
            Ok(())
        }

        // RDS Actions
        ("rds", "start_db_instance") => {
            clients.http.query_request("rds", "StartDBInstance", &[
                ("DBInstanceIdentifier", resource_id)
            ]).await?;
            Ok(())
        }
        ("rds", "stop_db_instance") => {
            clients.http.query_request("rds", "StopDBInstance", &[
                ("DBInstanceIdentifier", resource_id)
            ]).await?;
            Ok(())
        }
        ("rds", "reboot_db_instance") => {
            clients.http.query_request("rds", "RebootDBInstance", &[
                ("DBInstanceIdentifier", resource_id)
            ]).await?;
            Ok(())
        }
        ("rds", "delete_db_instance") => {
            clients.http.query_request("rds", "DeleteDBInstance", &[
                ("DBInstanceIdentifier", resource_id),
                ("SkipFinalSnapshot", "true")
            ]).await?;
            Ok(())
        }

        // ECS Actions
        ("ecs", "delete_cluster") => {
            clients.http.json_request("ecs", "DeleteCluster", &json!({
                "cluster": resource_id
            }).to_string()).await?;
            Ok(())
        }
        ("ecs", "delete_service") => {
            let parts: Vec<&str> = resource_id.split('/').collect();
            if parts.len() >= 2 {
                let cluster = parts[parts.len() - 2];
                clients.http.json_request("ecs", "DeleteService", &json!({
                    "cluster": cluster,
                    "service": resource_id,
                    "force": true
                }).to_string()).await?;
            }
            Ok(())
        }
        ("ecs", "stop_task") => {
            let parts: Vec<&str> = resource_id.split('/').collect();
            if parts.len() >= 2 {
                let cluster = parts[parts.len() - 2];
                clients.http.json_request("ecs", "StopTask", &json!({
                    "cluster": cluster,
                    "task": resource_id
                }).to_string()).await?;
            }
            Ok(())
        }

        // EKS Actions
        ("eks", "delete_cluster") => {
            clients.http.rest_json_request(
                "eks",
                "DELETE",
                &format!("/clusters/{}", resource_id),
                None
            ).await?;
            Ok(())
        }

        // S3 Actions
        ("s3", "delete_bucket") => {
            clients.http.rest_xml_request(
                "s3",
                "DELETE",
                &format!("/{}", resource_id),
                None
            ).await?;
            Ok(())
        }

        // DynamoDB Actions
        ("dynamodb", "delete_table") => {
            clients.http.json_request("dynamodb", "DeleteTable", &json!({
                "TableName": resource_id
            }).to_string()).await?;
            Ok(())
        }

        // SQS Actions
        ("sqs", "purge_queue") => {
            clients.http.query_request("sqs", "PurgeQueue", &[
                ("QueueUrl", resource_id)
            ]).await?;
            Ok(())
        }
        ("sqs", "delete_queue") => {
            clients.http.query_request("sqs", "DeleteQueue", &[
                ("QueueUrl", resource_id)
            ]).await?;
            Ok(())
        }

        // SNS Actions
        ("sns", "delete_topic") => {
            clients.http.query_request("sns", "DeleteTopic", &[
                ("TopicArn", resource_id)
            ]).await?;
            Ok(())
        }

        // CloudFormation Actions
        ("cloudformation", "delete_stack") => {
            clients.http.query_request("cloudformation", "DeleteStack", &[
                ("StackName", resource_id)
            ]).await?;
            Ok(())
        }

        // Secrets Manager Actions
        ("secretsmanager", "rotate_secret") => {
            clients.http.json_request("secretsmanager", "RotateSecret", &json!({
                "SecretId": resource_id
            }).to_string()).await?;
            Ok(())
        }
        ("secretsmanager", "delete_secret") => {
            clients.http.json_request("secretsmanager", "DeleteSecret", &json!({
                "SecretId": resource_id,
                "ForceDeleteWithoutRecovery": true
            }).to_string()).await?;
            Ok(())
        }

        // Auto Scaling Actions
        ("autoscaling", "delete_auto_scaling_group") => {
            clients.http.query_request("autoscaling", "DeleteAutoScalingGroup", &[
                ("AutoScalingGroupName", resource_id),
                ("ForceDelete", "true")
            ]).await?;
            Ok(())
        }

        // ELBv2 Actions
        ("elbv2", "delete_load_balancer") => {
            clients.http.query_request("elbv2", "DeleteLoadBalancer", &[
                ("LoadBalancerArn", resource_id)
            ]).await?;
            Ok(())
        }
        ("elbv2", "delete_listener") => {
            clients.http.query_request("elbv2", "DeleteListener", &[
                ("ListenerArn", resource_id)
            ]).await?;
            Ok(())
        }
        ("elbv2", "delete_rule") => {
            clients.http.query_request("elbv2", "DeleteRule", &[
                ("RuleArn", resource_id)
            ]).await?;
            Ok(())
        }
        ("elbv2", "delete_target_group") => {
            clients.http.query_request("elbv2", "DeleteTargetGroup", &[
                ("TargetGroupArn", resource_id)
            ]).await?;
            Ok(())
        }
        ("elbv2", "deregister_targets") => {
            // resource_id format: "target_group_arn|target_id:port"
            // For simplicity, we'll just use the resource_id as target_group_arn for now
            // The actual target deregistration would need more complex handling
            clients.http.query_request("elbv2", "DeregisterTargets", &[
                ("TargetGroupArn", resource_id),
                ("Targets.member.1.Id", resource_id)
            ]).await?;
            Ok(())
        }

        _ => Err(anyhow!("Unknown action: {}.{}", service, action)),
    }
}

// =============================================================================
// Describe Functions (single resource details)
// =============================================================================

/// Fetch full details for a single resource by ID
pub async fn describe_resource(
    resource_key: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<Value> {
    tracing::debug!("Describing resource: {} with id: {}", resource_key, resource_id);
    
    match resource_key {
        "ec2-instances" => {
            let xml = clients.http.query_request("ec2", "DescribeInstances", &[
                ("InstanceId.1", resource_id)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            // Navigate to the instance data
            if let Some(reservations) = json.pointer("/DescribeInstancesResponse/reservationSet/item") {
                let reservation = match reservations {
                    Value::Array(arr) => arr.first().cloned(),
                    obj @ Value::Object(_) => Some(obj.clone()),
                    _ => None,
                };
                
                if let Some(res) = reservation {
                    if let Some(instance) = res.pointer("/instancesSet/item") {
                        let instance = match instance {
                            Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
                            obj @ Value::Object(_) => obj.clone(),
                            _ => Value::Null,
                        };
                        return Ok(instance);
                    }
                }
            }
            Err(anyhow!("Instance not found"))
        }
        
        "s3-buckets" => {
            // S3 doesn't have a single describe API, so we fetch multiple properties
            let mut result = json!({
                "BucketName": resource_id,
            });
            
            // Get bucket location first (this determines the region for other calls)
            let bucket_region = clients.http.get_bucket_region(resource_id).await
                .unwrap_or_else(|_| "us-east-1".to_string());
            result["Region"] = json!(&bucket_region);
            
            // Get bucket versioning (using the correct regional endpoint)
            if let Ok(xml) = clients.http.rest_xml_request_s3_bucket(
                "GET",
                resource_id,
                "?versioning",
                None,
                &bucket_region
            ).await {
                if let Ok(json) = xml_to_json(&xml) {
                    let status = json.pointer("/VersioningConfiguration/Status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Disabled");
                    result["Versioning"] = json!(status);
                }
            }
            
            // Get bucket encryption (using the correct regional endpoint)
            if let Ok(xml) = clients.http.rest_xml_request_s3_bucket(
                "GET",
                resource_id,
                "?encryption",
                None,
                &bucket_region
            ).await {
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
        
        "lambda-functions" => {
            let response = clients.http.rest_json_request(
                "lambda",
                "GET",
                &format!("/2015-03-31/functions/{}", resource_id),
                None
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json)
        }
        
        "rds-instances" => {
            let xml = clients.http.query_request("rds", "DescribeDBInstances", &[
                ("DBInstanceIdentifier", resource_id)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            if let Some(instances) = json.pointer("/DescribeDBInstancesResponse/DescribeDBInstancesResult/DBInstances/DBInstance") {
                let instance = match instances {
                    Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
                    obj @ Value::Object(_) => obj.clone(),
                    _ => Value::Null,
                };
                return Ok(instance);
            }
            Err(anyhow!("RDS instance not found"))
        }
        
        "iam-users" => {
            let xml = clients.http.query_request("iam", "GetUser", &[
                ("UserName", resource_id)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            if let Some(user) = json.pointer("/GetUserResponse/GetUserResult/User") {
                return Ok(user.clone());
            }
            Err(anyhow!("IAM user not found"))
        }
        
        "iam-roles" => {
            let xml = clients.http.query_request("iam", "GetRole", &[
                ("RoleName", resource_id)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            if let Some(role) = json.pointer("/GetRoleResponse/GetRoleResult/Role") {
                return Ok(role.clone());
            }
            Err(anyhow!("IAM role not found"))
        }
        
        "dynamodb-tables" => {
            let response = clients.http.json_request(
                "dynamodb",
                "DescribeTable",
                &json!({ "TableName": resource_id }).to_string()
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json.get("Table").cloned().unwrap_or(json))
        }
        
        "eks-clusters" => {
            let response = clients.http.rest_json_request(
                "eks",
                "GET",
                &format!("/clusters/{}", resource_id),
                None
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json.get("cluster").cloned().unwrap_or(json))
        }
        
        "ecs-clusters" => {
            let response = clients.http.json_request(
                "ecs",
                "DescribeClusters",
                &json!({ "clusters": [resource_id] }).to_string()
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            if let Some(clusters) = json.get("clusters").and_then(|c| c.as_array()) {
                if let Some(cluster) = clusters.first() {
                    return Ok(cluster.clone());
                }
            }
            Err(anyhow!("ECS cluster not found"))
        }
        
        "secretsmanager-secrets" => {
            let response = clients.http.json_request(
                "secretsmanager",
                "DescribeSecret",
                &json!({ "SecretId": resource_id }).to_string()
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json)
        }
        
        "kms-keys" => {
            let response = clients.http.json_request(
                "kms",
                "DescribeKey",
                &json!({ "KeyId": resource_id }).to_string()
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json.get("KeyMetadata").cloned().unwrap_or(json))
        }
        
        "elbv2-load-balancers" => {
            let xml = clients.http.query_request("elbv2", "DescribeLoadBalancers", &[
                ("LoadBalancerArns.member.1", resource_id)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            if let Some(lbs) = json.pointer("/DescribeLoadBalancersResponse/DescribeLoadBalancersResult/LoadBalancers/member") {
                let lb = match lbs {
                    Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
                    obj @ Value::Object(_) => obj.clone(),
                    _ => Value::Null,
                };
                return Ok(lb);
            }
            Err(anyhow!("Load balancer not found"))
        }
        
        "elbv2-target-groups" => {
            let xml = clients.http.query_request("elbv2", "DescribeTargetGroups", &[
                ("TargetGroupArns.member.1", resource_id)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            if let Some(tgs) = json.pointer("/DescribeTargetGroupsResponse/DescribeTargetGroupsResult/TargetGroups/member") {
                let tg = match tgs {
                    Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
                    obj @ Value::Object(_) => obj.clone(),
                    _ => Value::Null,
                };
                return Ok(tg);
            }
            Err(anyhow!("Target group not found"))
        }
        
        // Default: return an error indicating describe is not implemented
        _ => {
            tracing::debug!("No describe implementation for {}, falling back to list data", resource_key);
            Err(anyhow!("Describe not implemented for {}", resource_key))
        }
    }
}

// =============================================================================
// List/Describe Functions (read operations)
// =============================================================================

/// Invoke an AWS API method and return the response as JSON.
pub async fn invoke_sdk(
    service: &str,
    method: &str,
    clients: &AwsClients,
    params: &Value,
) -> Result<Value> {
    match (service, method) {
        // =====================================================================
        // IAM Operations (Query protocol, global service)
        // =====================================================================
        ("iam", "list_users") => {
            let xml = clients.http.query_request("iam", "ListUsers", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let users = extract_iam_list(&json, "Users", "member");
            let result: Vec<Value> = users.iter().map(|u| {
                json!({
                    "UserId": u.get("UserId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "UserName": u.get("UserName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": u.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Path": u.get("Path").and_then(|v| v.as_str()).unwrap_or("/"),
                    "CreateDate": u.get("CreateDate").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "users": result }))
        }

        ("iam", "list_roles") => {
            let xml = clients.http.query_request("iam", "ListRoles", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let roles = extract_iam_list(&json, "Roles", "member");
            let result: Vec<Value> = roles.iter().map(|r| {
                json!({
                    "RoleId": r.get("RoleId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "RoleName": r.get("RoleName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": r.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Path": r.get("Path").and_then(|v| v.as_str()).unwrap_or("/"),
                    "CreateDate": r.get("CreateDate").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": r.get("Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "roles": result }))
        }

        ("iam", "list_policies") => {
            let scope = params.get("scope").and_then(|v| v.as_str()).unwrap_or("Local");
            let xml = clients.http.query_request("iam", "ListPolicies", &[
                ("Scope", scope)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let policies = extract_iam_list(&json, "Policies", "member");
            let result: Vec<Value> = policies.iter().map(|p| {
                json!({
                    "PolicyId": p.get("PolicyId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "PolicyName": p.get("PolicyName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": p.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Path": p.get("Path").and_then(|v| v.as_str()).unwrap_or("/"),
                    "CreateDate": p.get("CreateDate").and_then(|v| v.as_str()).unwrap_or("-"),
                    "AttachmentCount": p.get("AttachmentCount").and_then(|v| v.as_str()).unwrap_or("0"),
                    "IsAttachable": if p.get("IsAttachable").and_then(|v| v.as_str()) == Some("true") { "Yes" } else { "No" },
                })
            }).collect();
            
            Ok(json!({ "policies": result }))
        }

        ("iam", "list_groups") => {
            let xml = clients.http.query_request("iam", "ListGroups", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let groups = extract_iam_list(&json, "Groups", "member");
            let result: Vec<Value> = groups.iter().map(|g| {
                json!({
                    "GroupId": g.get("GroupId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "GroupName": g.get("GroupName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": g.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Path": g.get("Path").and_then(|v| v.as_str()).unwrap_or("/"),
                    "CreateDate": g.get("CreateDate").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "groups": result }))
        }

        ("iam", "list_attached_user_policies") => {
            let user_name = extract_param(params, "user_name");
            let xml = clients.http.query_request("iam", "ListAttachedUserPolicies", &[
                ("UserName", &user_name)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let policies = extract_iam_list(&json, "AttachedPolicies", "member");
            let result: Vec<Value> = policies.iter().map(|p| {
                json!({
                    "PolicyName": p.get("PolicyName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "PolicyArn": p.get("PolicyArn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "attached_policies": result }))
        }

        ("iam", "list_groups_for_user") => {
            let user_name = extract_param(params, "user_name");
            let xml = clients.http.query_request("iam", "ListGroupsForUser", &[
                ("UserName", &user_name)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let groups = extract_iam_list(&json, "Groups", "member");
            let result: Vec<Value> = groups.iter().map(|g| {
                json!({
                    "GroupId": g.get("GroupId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "GroupName": g.get("GroupName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": g.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "groups": result }))
        }

        ("iam", "list_access_keys") => {
            let user_name = extract_param(params, "user_name");
            let xml = clients.http.query_request("iam", "ListAccessKeys", &[
                ("UserName", &user_name)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let keys = extract_iam_list(&json, "AccessKeyMetadata", "member");
            let result: Vec<Value> = keys.iter().map(|k| {
                json!({
                    "AccessKeyId": k.get("AccessKeyId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Status": k.get("Status").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CreateDate": k.get("CreateDate").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "access_key_metadata": result }))
        }

        ("iam", "list_attached_role_policies") => {
            let role_name = extract_param(params, "role_name");
            let xml = clients.http.query_request("iam", "ListAttachedRolePolicies", &[
                ("RoleName", &role_name)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let policies = extract_iam_list(&json, "AttachedPolicies", "member");
            let result: Vec<Value> = policies.iter().map(|p| {
                json!({
                    "PolicyName": p.get("PolicyName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "PolicyArn": p.get("PolicyArn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "attached_policies": result }))
        }

        ("iam", "get_group") => {
            let group_name = extract_param(params, "group_name");
            let xml = clients.http.query_request("iam", "GetGroup", &[
                ("GroupName", &group_name)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let users = extract_iam_list(&json, "Users", "member");
            let result: Vec<Value> = users.iter().map(|u| {
                json!({
                    "UserId": u.get("UserId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "UserName": u.get("UserName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": u.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "users": result }))
        }

        // =====================================================================
        // EC2 Operations (Query protocol)
        // =====================================================================
        ("ec2", "describe_instances") => {
            let xml = clients.http.query_request("ec2", "DescribeInstances", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let mut instances: Vec<Value> = Vec::new();
            
            // Navigate: DescribeInstancesResponse > reservationSet > item > instancesSet > item
            if let Some(reservations) = json.pointer("/DescribeInstancesResponse/reservationSet/item") {
                let reservation_list = match reservations {
                    Value::Array(arr) => arr.clone(),
                    obj @ Value::Object(_) => vec![obj.clone()],
                    _ => vec![],
                };
                
                for reservation in reservation_list {
                    if let Some(instance_set) = reservation.pointer("/instancesSet/item") {
                        let instance_list = match instance_set {
                            Value::Array(arr) => arr.clone(),
                            obj @ Value::Object(_) => vec![obj.clone()],
                            _ => vec![],
                        };
                        
                        for instance in instance_list {
                            let tags = extract_tags(&instance);
                            instances.push(json!({
                                "InstanceId": instance.pointer("/instanceId").and_then(|v| v.as_str()).unwrap_or("-"),
                                "InstanceType": instance.pointer("/instanceType").and_then(|v| v.as_str()).unwrap_or("-"),
                                "State": instance.pointer("/instanceState/name").and_then(|v| v.as_str()).unwrap_or("-"),
                                "AvailabilityZone": instance.pointer("/placement/availabilityZone").and_then(|v| v.as_str()).unwrap_or("-"),
                                "PublicIpAddress": instance.pointer("/ipAddress").and_then(|v| v.as_str()).unwrap_or("-"),
                                "PrivateIpAddress": instance.pointer("/privateIpAddress").and_then(|v| v.as_str()).unwrap_or("-"),
                                "LaunchTime": instance.pointer("/launchTime").and_then(|v| v.as_str()).unwrap_or("-"),
                                "Tags": tags,
                            }));
                        }
                    }
                }
            }
            
            Ok(json!({ "reservations": instances }))
        }

        ("ec2", "describe_vpcs") => {
            let xml = clients.http.query_request("ec2", "DescribeVpcs", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let vpcs = extract_ec2_list(&json, "vpcSet");
            let result: Vec<Value> = vpcs.iter().map(|vpc| {
                let tags = extract_tags(vpc);
                json!({
                    "VpcId": vpc.pointer("/vpcId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "State": vpc.pointer("/state").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CidrBlock": vpc.pointer("/cidrBlock").and_then(|v| v.as_str()).unwrap_or("-"),
                    "IsDefault": if vpc.pointer("/isDefault").and_then(|v| v.as_str()) == Some("true") { "Yes" } else { "No" },
                    "InstanceTenancy": vpc.pointer("/instanceTenancy").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Tags": tags,
                })
            }).collect();
            
            Ok(json!({ "vpcs": result }))
        }

        ("ec2", "describe_subnets") => {
            let mut query_params: Vec<(&str, &str)> = vec![];
            let vpc_id_str: String;
            
            if let Some(vpc_ids) = params.get("vpc_ids").and_then(|v| v.as_array()) {
                if let Some(first_vpc) = vpc_ids.first().and_then(|v| v.as_str()) {
                    vpc_id_str = first_vpc.to_string();
                    query_params.push(("Filter.1.Name", "vpc-id"));
                    query_params.push(("Filter.1.Value.1", &vpc_id_str));
                }
            }
            
            let xml = clients.http.query_request("ec2", "DescribeSubnets", &query_params).await?;
            let json = xml_to_json(&xml)?;
            
            let subnets = extract_ec2_list(&json, "subnetSet");
            let result: Vec<Value> = subnets.iter().map(|subnet| {
                let tags = extract_tags(subnet);
                json!({
                    "SubnetId": subnet.pointer("/subnetId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "VpcId": subnet.pointer("/vpcId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "State": subnet.pointer("/state").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CidrBlock": subnet.pointer("/cidrBlock").and_then(|v| v.as_str()).unwrap_or("-"),
                    "AvailabilityZone": subnet.pointer("/availabilityZone").and_then(|v| v.as_str()).unwrap_or("-"),
                    "AvailableIpAddressCount": subnet.pointer("/availableIpAddressCount").and_then(|v| v.as_str()).unwrap_or("0"),
                    "Tags": tags,
                })
            }).collect();
            
            Ok(json!({ "subnets": result }))
        }

        ("ec2", "describe_security_groups") => {
            let mut query_params: Vec<(&str, &str)> = vec![];
            let vpc_id_str: String;
            
            if let Some(vpc_ids) = params.get("vpc_ids").and_then(|v| v.as_array()) {
                if let Some(first_vpc) = vpc_ids.first().and_then(|v| v.as_str()) {
                    vpc_id_str = first_vpc.to_string();
                    query_params.push(("Filter.1.Name", "vpc-id"));
                    query_params.push(("Filter.1.Value.1", &vpc_id_str));
                }
            }
            
            let xml = clients.http.query_request("ec2", "DescribeSecurityGroups", &query_params).await?;
            let json = xml_to_json(&xml)?;
            
            let groups = extract_ec2_list(&json, "securityGroupInfo");
            let result: Vec<Value> = groups.iter().map(|sg| {
                json!({
                    "GroupId": sg.pointer("/groupId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "GroupName": sg.pointer("/groupName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "VpcId": sg.pointer("/vpcId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": sg.pointer("/groupDescription").and_then(|v| v.as_str()).unwrap_or("-"),
                    "OwnerId": sg.pointer("/ownerId").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "security_groups": result }))
        }

        // =====================================================================
        // S3 Operations (REST-XML)
        // =====================================================================
        ("s3", "list_buckets") => {
            let xml = clients.http.rest_xml_request("s3", "GET", "/", None).await?;
            let json = xml_to_json(&xml)?;
            
            let buckets_data = json.pointer("/ListAllMyBucketsResult/Buckets/Bucket");
            let bucket_list = match buckets_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = bucket_list.iter().map(|b| {
                json!({
                    "Name": b.pointer("/Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CreationDate": b.pointer("/CreationDate").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "buckets": result }))
        }
        
        ("s3", "list_objects_v2") => {
            // Get bucket name from params
            let bucket = params.get("bucket_names")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Bucket name required"))?;
            
            // Get prefix for folder navigation (optional)
            // Can be either a string or array (from ResourceFilter)
            let prefix = params.get("prefix")
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else if let Some(arr) = v.as_array() {
                        arr.first().and_then(|v| v.as_str()).unwrap_or("").to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            
            // First, get the bucket's region (S3 buckets are region-specific)
            let bucket_region = clients.http.get_bucket_region(bucket).await?;
            debug!("Bucket {} is in region {}", bucket, bucket_region);
            
            let path = if prefix.is_empty() {
                "?list-type=2&delimiter=/".to_string()
            } else {
                format!("?list-type=2&delimiter=/&prefix={}", urlencoding::encode(&prefix))
            };
            
            let xml = clients.http.rest_xml_request_s3_bucket("GET", bucket, &path, None, &bucket_region).await?;
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
                    let display_name = prefix_val.trim_end_matches('/').rsplit('/').next().unwrap_or(prefix_val);
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
                    // Skip if key equals prefix (the folder itself)
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

        // =====================================================================
        // Lambda Operations (REST-JSON)
        // =====================================================================
        ("lambda", "list_functions") => {
            let response = clients.http.rest_json_request(
                "lambda",
                "GET",
                "/2015-03-31/functions",
                None
            ).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let functions = json.get("Functions").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = functions.iter().map(|f| {
                json!({
                    "FunctionName": f.get("FunctionName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Runtime": f.get("Runtime").and_then(|v| v.as_str()).unwrap_or("-"),
                    "MemorySize": f.get("MemorySize").and_then(|v| v.as_i64()).unwrap_or(0),
                    "LastModified": f.get("LastModified").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": f.get("Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "functions": result }))
        }

        // =====================================================================
        // RDS Operations (Query protocol)
        // =====================================================================
        ("rds", "describe_db_instances") => {
            let xml = clients.http.query_request("rds", "DescribeDBInstances", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let instances = extract_rds_list(&json, "DBInstances", "DBInstance");
            let result: Vec<Value> = instances.iter().map(|db| {
                json!({
                    "DBInstanceIdentifier": db.pointer("/DBInstanceIdentifier").and_then(|v| v.as_str()).unwrap_or("-"),
                    "DBInstanceStatus": db.pointer("/DBInstanceStatus").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Engine": db.pointer("/Engine").and_then(|v| v.as_str()).unwrap_or("-"),
                    "DBInstanceClass": db.pointer("/DBInstanceClass").and_then(|v| v.as_str()).unwrap_or("-"),
                    "AvailabilityZone": db.pointer("/AvailabilityZone").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Endpoint": db.pointer("/Endpoint/Address").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "db_instances": result }))
        }

        ("rds", "describe_db_snapshots") => {
            let db_id = extract_param(params, "db_instance_identifier");
            let mut query_params = vec![];
            if !db_id.is_empty() {
                query_params.push(("DBInstanceIdentifier", db_id.as_str()));
            }
            
            let xml = clients.http.query_request("rds", "DescribeDBSnapshots", &query_params).await?;
            let json = xml_to_json(&xml)?;
            
            let snapshots = extract_rds_list(&json, "DBSnapshots", "DBSnapshot");
            let result: Vec<Value> = snapshots.iter().map(|snap| {
                json!({
                    "DBSnapshotIdentifier": snap.pointer("/DBSnapshotIdentifier").and_then(|v| v.as_str()).unwrap_or("-"),
                    "DBInstanceIdentifier": snap.pointer("/DBInstanceIdentifier").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Status": snap.pointer("/Status").and_then(|v| v.as_str()).unwrap_or("-"),
                    "SnapshotType": snap.pointer("/SnapshotType").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Engine": snap.pointer("/Engine").and_then(|v| v.as_str()).unwrap_or("-"),
                    "AllocatedStorage": snap.pointer("/AllocatedStorage").and_then(|v| v.as_str()).unwrap_or("0"),
                    "SnapshotCreateTime": snap.pointer("/SnapshotCreateTime").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "db_snapshots": result }))
        }

        // =====================================================================
        // DynamoDB Operations (JSON protocol)
        // =====================================================================
        ("dynamodb", "list_tables") => {
            let response = clients.http.json_request("dynamodb", "ListTables", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let tables = json.get("TableNames").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = tables.iter().map(|name| {
                json!({
                    "TableName": name.as_str().unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "table_names": result }))
        }

        // =====================================================================
        // ECS Operations (JSON protocol)
        // =====================================================================
        ("ecs", "list_clusters_with_details") => {
            // List clusters
            let list_response = clients.http.json_request("ecs", "ListClusters", "{}").await?;
            let list_json: Value = serde_json::from_str(&list_response)?;
            let cluster_arns = list_json.get("clusterArns").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            
            if cluster_arns.is_empty() {
                return Ok(json!({ "clusters": [] }));
            }
            
            // Describe clusters
            let desc_response = clients.http.json_request("ecs", "DescribeClusters", &json!({
                "clusters": cluster_arns
            }).to_string()).await?;
            let desc_json: Value = serde_json::from_str(&desc_response)?;
            
            let clusters = desc_json.get("clusters").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = clusters.iter().map(|c| {
                json!({
                    "clusterArn": c.get("clusterArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "clusterName": c.get("clusterName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "status": c.get("status").and_then(|v| v.as_str()).unwrap_or("-"),
                    "runningTasksCount": c.get("runningTasksCount").and_then(|v| v.as_i64()).unwrap_or(0),
                    "registeredContainerInstancesCount": c.get("registeredContainerInstancesCount").and_then(|v| v.as_i64()).unwrap_or(0),
                })
            }).collect();
            
            Ok(json!({ "clusters": result }))
        }

        ("ecs", "list_services_with_details") => {
            let cluster = extract_param(params, "cluster");
            if cluster.is_empty() {
                return Ok(json!({ "services": [] }));
            }
            
            let list_response = clients.http.json_request("ecs", "ListServices", &json!({
                "cluster": cluster
            }).to_string()).await?;
            let list_json: Value = serde_json::from_str(&list_response)?;
            let service_arns = list_json.get("serviceArns").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            
            if service_arns.is_empty() {
                return Ok(json!({ "services": [] }));
            }
            
            let desc_response = clients.http.json_request("ecs", "DescribeServices", &json!({
                "cluster": cluster,
                "services": service_arns
            }).to_string()).await?;
            let desc_json: Value = serde_json::from_str(&desc_response)?;
            
            let services = desc_json.get("services").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = services.iter().map(|s| {
                json!({
                    "serviceArn": s.get("serviceArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "serviceName": s.get("serviceName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "status": s.get("status").and_then(|v| v.as_str()).unwrap_or("-"),
                    "desiredCount": s.get("desiredCount").and_then(|v| v.as_i64()).unwrap_or(0),
                    "runningCount": s.get("runningCount").and_then(|v| v.as_i64()).unwrap_or(0),
                    "launchType": s.get("launchType").and_then(|v| v.as_str()).unwrap_or("-"),
                    "clusterArn": s.get("clusterArn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "services": result }))
        }

        ("ecs", "list_tasks_with_details") => {
            let cluster = extract_param(params, "cluster");
            if cluster.is_empty() {
                return Ok(json!({ "tasks": [] }));
            }
            
            let list_response = clients.http.json_request("ecs", "ListTasks", &json!({
                "cluster": cluster
            }).to_string()).await?;
            let list_json: Value = serde_json::from_str(&list_response)?;
            let task_arns = list_json.get("taskArns").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            
            if task_arns.is_empty() {
                return Ok(json!({ "tasks": [] }));
            }
            
            let desc_response = clients.http.json_request("ecs", "DescribeTasks", &json!({
                "cluster": cluster,
                "tasks": task_arns
            }).to_string()).await?;
            let desc_json: Value = serde_json::from_str(&desc_response)?;
            
            let tasks = desc_json.get("tasks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = tasks.iter().map(|t| {
                json!({
                    "taskArn": t.get("taskArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "lastStatus": t.get("lastStatus").and_then(|v| v.as_str()).unwrap_or("-"),
                    "desiredStatus": t.get("desiredStatus").and_then(|v| v.as_str()).unwrap_or("-"),
                    "cpu": t.get("cpu").and_then(|v| v.as_str()).unwrap_or("-"),
                    "memory": t.get("memory").and_then(|v| v.as_str()).unwrap_or("-"),
                    "clusterArn": t.get("clusterArn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "tasks": result }))
        }

        // =====================================================================
        // SQS Operations (Query protocol)
        // =====================================================================
        ("sqs", "list_queues") => {
            let xml = clients.http.query_request("sqs", "ListQueues", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let queue_urls = json.pointer("/ListQueuesResponse/ListQueuesResult/QueueUrl");
            let queue_list = match queue_urls {
                Some(Value::Array(arr)) => arr.clone(),
                Some(Value::String(s)) => vec![Value::String(s.clone())],
                _ => vec![],
            };
            
            let result: Vec<Value> = queue_list.iter().map(|url| {
                json!({
                    "QueueUrl": url.as_str().unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "queue_urls": result }))
        }

        // =====================================================================
        // SNS Operations (Query protocol)
        // =====================================================================
        ("sns", "list_topics") => {
            let xml = clients.http.query_request("sns", "ListTopics", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let topics_data = json.pointer("/ListTopicsResponse/ListTopicsResult/Topics/member");
            let topic_list = match topics_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = topic_list.iter().map(|t| {
                json!({
                    "TopicArn": t.pointer("/TopicArn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "topics": result }))
        }

        // =====================================================================
        // CloudFormation Operations (Query protocol)
        // =====================================================================
        ("cloudformation", "describe_stacks") => {
            let xml = clients.http.query_request("cloudformation", "DescribeStacks", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let stacks_data = json.pointer("/DescribeStacksResponse/DescribeStacksResult/Stacks/member");
            let stack_list = match stacks_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = stack_list.iter().map(|stack| {
                json!({
                    "StackName": stack.pointer("/StackName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "StackId": stack.pointer("/StackId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "StackStatus": stack.pointer("/StackStatus").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CreationTime": stack.pointer("/CreationTime").and_then(|v| v.as_str()).unwrap_or("-"),
                    "LastUpdatedTime": stack.pointer("/LastUpdatedTime").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": stack.pointer("/Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "stacks": result }))
        }

        // =====================================================================
        // CloudWatch Logs Operations (JSON protocol)
        // =====================================================================
        ("cloudwatchlogs", "describe_log_groups") => {
            let response = clients.http.json_request("logs", "DescribeLogGroups", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let log_groups = json.get("logGroups").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = log_groups.iter().map(|lg| {
                json!({
                    "logGroupName": lg.get("logGroupName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "logGroupArn": lg.get("arn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "storedBytes": lg.get("storedBytes").and_then(|v| v.as_i64()).unwrap_or(0),
                    "retentionInDays": lg.get("retentionInDays").map(|v| v.to_string()).unwrap_or("Never".to_string()),
                    "creationTime": lg.get("creationTime").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "log_groups": result }))
        }

        ("cloudwatchlogs", "describe_log_streams") => {
            let log_group_name = extract_param(params, "log_group_name");
            
            // Build request with pagination support
            let page_token = params.get("_page_token").and_then(|v| v.as_str());
            let request_body = if let Some(token) = page_token {
                json!({
                    "logGroupName": log_group_name,
                    "orderBy": "LastEventTime",
                    "descending": true,
                    "limit": 50,
                    "nextToken": token
                }).to_string()
            } else {
                json!({
                    "logGroupName": log_group_name,
                    "orderBy": "LastEventTime",
                    "descending": true,
                    "limit": 50
                }).to_string()
            };
            
            let response = clients.http.json_request("logs", "DescribeLogStreams", &request_body).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let log_streams = json.get("logStreams").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = log_streams.iter().map(|ls| {
                // Format timestamps as human-readable dates
                let last_event = ls.get("lastEventTimestamp")
                    .and_then(|v| v.as_i64())
                    .map(|ts| format_epoch_millis(ts))
                    .unwrap_or("-".to_string());
                let first_event = ls.get("firstEventTimestamp")
                    .and_then(|v| v.as_i64())
                    .map(|ts| format_epoch_millis(ts))
                    .unwrap_or("-".to_string());
                    
                json!({
                    "logStreamName": ls.get("logStreamName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "logGroupName": log_group_name,
                    "lastEventTime": last_event,
                    "firstEventTime": first_event,
                    "storedBytes": format_bytes(ls.get("storedBytes").and_then(|v| v.as_u64()).unwrap_or(0)),
                    "lastEventTimestamp": ls.get("lastEventTimestamp").and_then(|v| v.as_i64()).unwrap_or(0),
                })
            }).collect();
            
            // Include next_token in response for pagination
            let next_token = json.get("nextToken").and_then(|v| v.as_str());
            let mut response = json!({ "log_streams": result });
            if let Some(token) = next_token {
                response["_next_token"] = json!(token);
            }
            
            Ok(response)
        }

        ("cloudwatchlogs", "get_log_events") => {
            let log_group_name = extract_param(params, "log_group_name");
            let log_stream_name = extract_param(params, "log_stream_name");
            let next_token = params.get("next_forward_token").and_then(|v| v.as_str());
            let start_time = params.get("start_time").and_then(|v| v.as_i64());
            
            let mut request = json!({
                "logGroupName": log_group_name,
                "logStreamName": log_stream_name,
                "startFromHead": false,
                "limit": 100
            });
            
            if let Some(token) = next_token {
                request["nextToken"] = json!(token);
            }
            if let Some(ts) = start_time {
                request["startTime"] = json!(ts);
            }
            
            let response = clients.http.json_request("logs", "GetLogEvents", &request.to_string()).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let events = json.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = events.iter().map(|ev| {
                json!({
                    "timestamp": ev.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0),
                    "message": ev.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                    "ingestionTime": ev.get("ingestionTime").and_then(|v| v.as_i64()).unwrap_or(0),
                })
            }).collect();
            
            Ok(json!({
                "events": result,
                "nextForwardToken": json.get("nextForwardToken").and_then(|v| v.as_str()),
                "nextBackwardToken": json.get("nextBackwardToken").and_then(|v| v.as_str())
            }))
        }

        // =====================================================================
        // Secrets Manager Operations (JSON protocol)
        // =====================================================================
        ("secretsmanager", "list_secrets") => {
            // Build request with pagination support
            let page_token = params.get("_page_token").and_then(|v| v.as_str());
            let request_body = if let Some(token) = page_token {
                json!({ "NextToken": token, "MaxResults": 100 }).to_string()
            } else {
                json!({ "MaxResults": 100 }).to_string()
            };
            
            let response = clients.http.json_request("secretsmanager", "ListSecrets", &request_body).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let secrets = json.get("SecretList").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = secrets.iter().map(|secret| {
                json!({
                    "Name": secret.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "ARN": secret.get("ARN").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": secret.get("Description").and_then(|v| v.as_str()).unwrap_or("-"),
                    "LastAccessedDate": secret.get("LastAccessedDate").map(|v| v.to_string()).unwrap_or("-".to_string()),
                    "LastChangedDate": secret.get("LastChangedDate").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            // Include next_token in response for pagination
            let next_token = json.get("NextToken").and_then(|v| v.as_str());
            let mut response = json!({ "secrets": result });
            if let Some(token) = next_token {
                response["_next_token"] = json!(token);
            }
            
            Ok(response)
        }

        // =====================================================================
        // SSM Operations (JSON protocol)
        // =====================================================================
        ("ssm", "describe_parameters") => {
            // Build request with pagination support
            let page_token = params.get("_page_token").and_then(|v| v.as_str());
            let request_body = if let Some(token) = page_token {
                json!({ "NextToken": token, "MaxResults": 50 }).to_string()
            } else {
                json!({ "MaxResults": 50 }).to_string()
            };
            
            let response = clients.http.json_request("ssm", "DescribeParameters", &request_body).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let parameters = json.get("Parameters").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = parameters.iter().map(|param| {
                json!({
                    "Name": param.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Type": param.get("Type").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Tier": param.get("Tier").and_then(|v| v.as_str()).unwrap_or("-"),
                    "LastModifiedDate": param.get("LastModifiedDate").map(|v| v.to_string()).unwrap_or("-".to_string()),
                    "Description": param.get("Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            // Include next_token in response for pagination
            let next_token = json.get("NextToken").and_then(|v| v.as_str());
            let mut response = json!({ "parameters": result });
            if let Some(token) = next_token {
                response["_next_token"] = json!(token);
            }
            
            Ok(response)
        }

        // =====================================================================
        // EKS Operations (REST-JSON)
        // =====================================================================
        ("eks", "list_clusters_with_details") => {
            let list_response = clients.http.rest_json_request("eks", "GET", "/clusters", None).await?;
            let list_json: Value = serde_json::from_str(&list_response)?;
            let cluster_names = list_json.get("clusters").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            
            if cluster_names.is_empty() {
                return Ok(json!({ "clusters": [] }));
            }
            
            let mut clusters: Vec<Value> = Vec::new();
            for name in cluster_names {
                if let Some(name_str) = name.as_str() {
                    if let Ok(desc_response) = clients.http.rest_json_request(
                        "eks",
                        "GET",
                        &format!("/clusters/{}", name_str),
                        None
                    ).await {
                        if let Ok(desc_json) = serde_json::from_str::<Value>(&desc_response) {
                            if let Some(cluster) = desc_json.get("cluster") {
                                clusters.push(json!({
                                    "name": cluster.get("name").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "arn": cluster.get("arn").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "status": cluster.get("status").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "version": cluster.get("version").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "endpoint": cluster.get("endpoint").and_then(|v| v.as_str()).unwrap_or("-"),
                                }));
                            }
                        }
                    }
                }
            }
            
            Ok(json!({ "clusters": clusters }))
        }

        // =====================================================================
        // API Gateway Operations (REST-JSON)
        // =====================================================================
        ("apigateway", "get_rest_apis") => {
            let response = clients.http.rest_json_request("apigateway", "GET", "/restapis", None).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let items = json.get("item").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = items.iter().map(|api| {
                json!({
                    "id": api.get("id").and_then(|v| v.as_str()).unwrap_or("-"),
                    "name": api.get("name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "description": api.get("description").and_then(|v| v.as_str()).unwrap_or("-"),
                    "createdDate": api.get("createdDate").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "items": result }))
        }

        // =====================================================================
        // Route53 Operations (REST-XML, global)
        // =====================================================================
        ("route53", "list_hosted_zones") => {
            let xml = clients.http.rest_xml_request("route53", "GET", "/2013-04-01/hostedzone", None).await?;
            let json = xml_to_json(&xml)?;
            
            let zones_data = json.pointer("/ListHostedZonesResponse/HostedZones/HostedZone");
            let zone_list = match zones_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = zone_list.iter().map(|zone| {
                let is_private = zone.pointer("/Config/PrivateZone").and_then(|v| v.as_str()) == Some("true");
                json!({
                    "Id": zone.pointer("/Id").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Name": zone.pointer("/Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "ResourceRecordSetCount": zone.pointer("/ResourceRecordSetCount").and_then(|v| v.as_str()).unwrap_or("0"),
                    "Config.PrivateZone": if is_private { "Private" } else { "Public" },
                })
            }).collect();
            
            Ok(json!({ "hosted_zones": result }))
        }

        // =====================================================================
        // ElastiCache Operations (Query protocol)
        // =====================================================================
        ("elasticache", "describe_cache_clusters") => {
            let xml = clients.http.query_request("elasticache", "DescribeCacheClusters", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let clusters_data = json.pointer("/DescribeCacheClustersResponse/DescribeCacheClustersResult/CacheClusters/CacheCluster");
            let cluster_list = match clusters_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = cluster_list.iter().map(|cluster| {
                json!({
                    "CacheClusterId": cluster.pointer("/CacheClusterId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CacheClusterStatus": cluster.pointer("/CacheClusterStatus").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Engine": cluster.pointer("/Engine").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CacheNodeType": cluster.pointer("/CacheNodeType").and_then(|v| v.as_str()).unwrap_or("-"),
                    "NumCacheNodes": cluster.pointer("/NumCacheNodes").and_then(|v| v.as_str()).unwrap_or("0"),
                })
            }).collect();
            
            Ok(json!({ "cache_clusters": result }))
        }

        // =====================================================================
        // STS Operations (Query protocol)
        // =====================================================================
        ("sts", "get_caller_identity") => {
            let xml = clients.http.query_request("sts", "GetCallerIdentity", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let result_path = json.pointer("/GetCallerIdentityResponse/GetCallerIdentityResult");
            let identity = json!({
                "Account": result_path.and_then(|r| r.pointer("/Account")).and_then(|v| v.as_str()).unwrap_or("-"),
                "UserId": result_path.and_then(|r| r.pointer("/UserId")).and_then(|v| v.as_str()).unwrap_or("-"),
                "Arn": result_path.and_then(|r| r.pointer("/Arn")).and_then(|v| v.as_str()).unwrap_or("-"),
            });
            
            Ok(json!({ "identity": [identity] }))
        }

        // =====================================================================
        // ECR Operations (JSON protocol)
        // =====================================================================
        ("ecr", "describe_repositories") => {
            let response = clients.http.json_request("ecr", "DescribeRepositories", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let repos = json.get("repositories").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = repos.iter().map(|repo| {
                json!({
                    "repositoryName": repo.get("repositoryName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "repositoryArn": repo.get("repositoryArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "repositoryUri": repo.get("repositoryUri").and_then(|v| v.as_str()).unwrap_or("-"),
                    "createdAt": repo.get("createdAt").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "repositories": result }))
        }

        // =====================================================================
        // KMS Operations (JSON protocol)
        // =====================================================================
        ("kms", "list_keys_with_details") => {
            let response = clients.http.json_request("kms", "ListKeys", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let keys_list = json.get("Keys").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let mut keys: Vec<Value> = Vec::new();
            
            for key in keys_list {
                if let Some(key_id) = key.get("KeyId").and_then(|v| v.as_str()) {
                    if let Ok(desc_response) = clients.http.json_request("kms", "DescribeKey", &json!({
                        "KeyId": key_id
                    }).to_string()).await {
                        if let Ok(desc_json) = serde_json::from_str::<Value>(&desc_response) {
                            if let Some(metadata) = desc_json.get("KeyMetadata") {
                                keys.push(json!({
                                    "KeyId": metadata.get("KeyId").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "KeyArn": metadata.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "KeyState": metadata.get("KeyState").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "KeyUsage": metadata.get("KeyUsage").and_then(|v| v.as_str()).unwrap_or("-"),
                                    "KeySpec": metadata.get("KeySpec").and_then(|v| v.as_str()).unwrap_or("-"),
                                }));
                            }
                        }
                    }
                }
            }
            
            Ok(json!({ "keys": keys }))
        }

        // =====================================================================
        // CloudFront Operations (REST-XML, global)
        // =====================================================================
        ("cloudfront", "list_distributions") => {
            let xml = clients.http.rest_xml_request("cloudfront", "GET", "/2020-05-31/distribution", None).await?;
            let json = xml_to_json(&xml)?;
            
            let items_data = json.pointer("/DistributionList/Items/DistributionSummary");
            let item_list = match items_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = item_list.iter().map(|dist| {
                json!({
                    "Id": dist.pointer("/Id").and_then(|v| v.as_str()).unwrap_or("-"),
                    "DomainName": dist.pointer("/DomainName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Status": dist.pointer("/Status").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Enabled": if dist.pointer("/Enabled").and_then(|v| v.as_str()) == Some("true") { "Yes" } else { "No" },
                })
            }).collect();
            
            Ok(json!({ "distributions": result }))
        }

        // =====================================================================
        // ACM Operations (JSON protocol)
        // =====================================================================
        ("acm", "list_certificates") => {
            let response = clients.http.json_request("acm", "ListCertificates", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let certs = json.get("CertificateSummaryList").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = certs.iter().map(|cert| {
                json!({
                    "DomainName": cert.get("DomainName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CertificateArn": cert.get("CertificateArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Status": cert.get("Status").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Type": cert.get("Type").and_then(|v| v.as_str()).unwrap_or("-"),
                    "InUse": if cert.get("InUse").and_then(|v| v.as_bool()).unwrap_or(false) { "Yes" } else { "No" },
                })
            }).collect();
            
            Ok(json!({ "certificates": result }))
        }

        // =====================================================================
        // EventBridge Operations (JSON protocol)
        // =====================================================================
        ("eventbridge", "list_rules") => {
            let response = clients.http.json_request("events", "ListRules", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let rules = json.get("Rules").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = rules.iter().map(|rule| {
                json!({
                    "Name": rule.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": rule.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "State": rule.get("State").and_then(|v| v.as_str()).unwrap_or("-"),
                    "EventBusName": rule.get("EventBusName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": rule.get("Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "rules": result }))
        }

        ("eventbridge", "list_event_buses") => {
            let response = clients.http.json_request("events", "ListEventBuses", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let buses = json.get("EventBuses").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = buses.iter().map(|bus| {
                json!({
                    "Name": bus.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Arn": bus.get("Arn").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "event_buses": result }))
        }

        // =====================================================================
        // CodePipeline Operations (JSON protocol)
        // =====================================================================
        ("codepipeline", "list_pipelines") => {
            let response = clients.http.json_request("codepipeline", "ListPipelines", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let pipelines = json.get("pipelines").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = pipelines.iter().map(|pipeline| {
                json!({
                    "name": pipeline.get("name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "version": pipeline.get("version").and_then(|v| v.as_i64()).unwrap_or(0),
                    "created": pipeline.get("created").map(|v| v.to_string()).unwrap_or("-".to_string()),
                    "updated": pipeline.get("updated").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "pipelines": result }))
        }

        // =====================================================================
        // CodeBuild Operations (JSON protocol)
        // =====================================================================
        ("codebuild", "list_projects_with_details") => {
            let list_response = clients.http.json_request("codebuild", "ListProjects", "{}").await?;
            let list_json: Value = serde_json::from_str(&list_response)?;
            let project_names = list_json.get("projects").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            
            if project_names.is_empty() {
                return Ok(json!({ "projects": [] }));
            }
            
            let batch_response = clients.http.json_request("codebuild", "BatchGetProjects", &json!({
                "names": project_names
            }).to_string()).await?;
            let batch_json: Value = serde_json::from_str(&batch_response)?;
            
            let projects = batch_json.get("projects").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = projects.iter().map(|proj| {
                json!({
                    "name": proj.get("name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "sourceType": proj.pointer("/source/type").and_then(|v| v.as_str()).unwrap_or("-"),
                    "created": proj.get("created").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "projects": result }))
        }

        // =====================================================================
        // Cognito Operations (JSON protocol)
        // =====================================================================
        ("cognitoidentityprovider", "list_user_pools") => {
            let response = clients.http.json_request("cognito-idp", "ListUserPools", &json!({
                "MaxResults": 60
            }).to_string()).await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let pools = json.get("UserPools").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = pools.iter().map(|pool| {
                json!({
                    "Id": pool.get("Id").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Name": pool.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Status": "-",
                    "CreationDate": pool.get("CreationDate").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "user_pools": result }))
        }

        // =====================================================================
        // CloudTrail Operations (JSON protocol)
        // =====================================================================
        ("cloudtrail", "describe_trails") => {
            let response = clients.http.json_request("cloudtrail", "DescribeTrails", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let trails = json.get("trailList").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = trails.iter().map(|trail| {
                json!({
                    "Name": trail.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "TrailARN": trail.get("TrailARN").and_then(|v| v.as_str()).unwrap_or("-"),
                    "S3BucketName": trail.get("S3BucketName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "IsMultiRegionTrail": if trail.get("IsMultiRegionTrail").and_then(|v| v.as_bool()).unwrap_or(false) { "Yes" } else { "No" },
                    "LogFileValidationEnabled": if trail.get("LogFileValidationEnabled").and_then(|v| v.as_bool()).unwrap_or(false) { "Yes" } else { "No" },
                })
            }).collect();
            
            Ok(json!({ "trails": result }))
        }

        // =====================================================================
        // Auto Scaling Operations (Query protocol)
        // =====================================================================
        ("autoscaling", "describe_auto_scaling_groups") => {
            let xml = clients.http.query_request("autoscaling", "DescribeAutoScalingGroups", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let groups_data = json.pointer("/DescribeAutoScalingGroupsResponse/DescribeAutoScalingGroupsResult/AutoScalingGroups/member");
            let group_list = match groups_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = group_list.iter().map(|asg| {
                json!({
                    "AutoScalingGroupName": asg.pointer("/AutoScalingGroupName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "MinSize": asg.pointer("/MinSize").and_then(|v| v.as_str()).unwrap_or("0"),
                    "MaxSize": asg.pointer("/MaxSize").and_then(|v| v.as_str()).unwrap_or("0"),
                    "DesiredCapacity": asg.pointer("/DesiredCapacity").and_then(|v| v.as_str()).unwrap_or("0"),
                    "InstanceCount": "0", // Would need to count instances
                    "AvailabilityZones": asg.pointer("/AvailabilityZones").map(|v| v.to_string()).unwrap_or("-".to_string()),
                })
            }).collect();
            
            Ok(json!({ "auto_scaling_groups": result }))
        }

        // =====================================================================
        // Athena Operations (JSON protocol)
        // =====================================================================
        ("athena", "list_work_groups") => {
            let response = clients.http.json_request("athena", "ListWorkGroups", "{}").await?;
            let json: Value = serde_json::from_str(&response)?;
            
            let workgroups = json.get("WorkGroups").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let result: Vec<Value> = workgroups.iter().map(|wg| {
                json!({
                    "Name": wg.get("Name").and_then(|v| v.as_str()).unwrap_or("-"),
                    "State": wg.get("State").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Description": wg.get("Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "work_groups": result }))
        }

        // =====================================================================
        // ELBv2 Operations (Query protocol)
        // =====================================================================
        ("elbv2", "describe_load_balancers") => {
            let xml = clients.http.query_request("elbv2", "DescribeLoadBalancers", &[]).await?;
            let json = xml_to_json(&xml)?;
            
            let lbs_data = json.pointer("/DescribeLoadBalancersResponse/DescribeLoadBalancersResult/LoadBalancers/member");
            let lb_list = match lbs_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = lb_list.iter().map(|lb| {
                let state = lb.pointer("/State/Code").and_then(|v| v.as_str()).unwrap_or("-");
                json!({
                    "LoadBalancerArn": lb.pointer("/LoadBalancerArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "LoadBalancerName": lb.pointer("/LoadBalancerName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "DNSName": lb.pointer("/DNSName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Type": lb.pointer("/Type").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Scheme": lb.pointer("/Scheme").and_then(|v| v.as_str()).unwrap_or("-"),
                    "State": state,
                    "VpcId": lb.pointer("/VpcId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "CreatedTime": lb.pointer("/CreatedTime").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "load_balancers": result }))
        }

        ("elbv2", "describe_listeners") => {
            let lb_arn = extract_param(params, "load_balancer_arn");
            if lb_arn.is_empty() {
                return Ok(json!({ "listeners": [] }));
            }
            
            let xml = clients.http.query_request("elbv2", "DescribeListeners", &[
                ("LoadBalancerArn", &lb_arn)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let listeners_data = json.pointer("/DescribeListenersResponse/DescribeListenersResult/Listeners/member");
            let listener_list = match listeners_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = listener_list.iter().map(|listener| {
                // Get the default action type
                let default_action = listener.pointer("/DefaultActions/member")
                    .and_then(|v| match v {
                        Value::Array(arr) => arr.first(),
                        obj @ Value::Object(_) => Some(obj),
                        _ => None,
                    })
                    .and_then(|a| a.pointer("/Type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                
                json!({
                    "ListenerArn": listener.pointer("/ListenerArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "LoadBalancerArn": listener.pointer("/LoadBalancerArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Port": listener.pointer("/Port").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Protocol": listener.pointer("/Protocol").and_then(|v| v.as_str()).unwrap_or("-"),
                    "SslPolicy": listener.pointer("/SslPolicy").and_then(|v| v.as_str()).unwrap_or("-"),
                    "DefaultActionType": default_action,
                })
            }).collect();
            
            Ok(json!({ "listeners": result }))
        }

        ("elbv2", "describe_rules") => {
            let listener_arn = extract_param(params, "listener_arn");
            if listener_arn.is_empty() {
                return Ok(json!({ "rules": [] }));
            }
            
            let xml = clients.http.query_request("elbv2", "DescribeRules", &[
                ("ListenerArn", &listener_arn)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let rules_data = json.pointer("/DescribeRulesResponse/DescribeRulesResult/Rules/member");
            let rule_list = match rules_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = rule_list.iter().map(|rule| {
                // Get the first action type and target group
                let action = rule.pointer("/Actions/member")
                    .and_then(|v| match v {
                        Value::Array(arr) => arr.first().cloned(),
                        obj @ Value::Object(_) => Some(obj.clone()),
                        _ => None,
                    });
                let action_type = action.as_ref()
                    .and_then(|a| a.pointer("/Type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let target_group_arn = action.as_ref()
                    .and_then(|a| a.pointer("/TargetGroupArn"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                
                // Summarize conditions
                let conditions = rule.pointer("/Conditions/member");
                let conditions_summary = match conditions {
                    Some(Value::Array(arr)) => {
                        arr.iter()
                            .filter_map(|c| c.pointer("/Field").and_then(|v| v.as_str()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                    Some(obj @ Value::Object(_)) => {
                        obj.pointer("/Field").and_then(|v| v.as_str()).unwrap_or("-").to_string()
                    }
                    _ => "-".to_string(),
                };
                
                json!({
                    "RuleArn": rule.pointer("/RuleArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Priority": rule.pointer("/Priority").and_then(|v| v.as_str()).unwrap_or("-"),
                    "IsDefault": if rule.pointer("/IsDefault").and_then(|v| v.as_str()) == Some("true") { "Yes" } else { "No" },
                    "ConditionsSummary": if conditions_summary.is_empty() { "-" } else { &conditions_summary },
                    "ActionType": action_type,
                    "TargetGroupArn": target_group_arn,
                })
            }).collect();
            
            Ok(json!({ "rules": result }))
        }

        ("elbv2", "describe_target_groups") => {
            let lb_arn = extract_param(params, "load_balancer_arn");
            let mut query_params: Vec<(&str, &str)> = vec![];
            
            if !lb_arn.is_empty() {
                query_params.push(("LoadBalancerArn", &lb_arn));
            }
            
            let xml = clients.http.query_request("elbv2", "DescribeTargetGroups", &query_params).await?;
            let json = xml_to_json(&xml)?;
            
            let tgs_data = json.pointer("/DescribeTargetGroupsResponse/DescribeTargetGroupsResult/TargetGroups/member");
            let tg_list = match tgs_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = tg_list.iter().map(|tg| {
                json!({
                    "TargetGroupArn": tg.pointer("/TargetGroupArn").and_then(|v| v.as_str()).unwrap_or("-"),
                    "TargetGroupName": tg.pointer("/TargetGroupName").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Protocol": tg.pointer("/Protocol").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Port": tg.pointer("/Port").and_then(|v| v.as_str()).unwrap_or("-"),
                    "VpcId": tg.pointer("/VpcId").and_then(|v| v.as_str()).unwrap_or("-"),
                    "TargetType": tg.pointer("/TargetType").and_then(|v| v.as_str()).unwrap_or("-"),
                    "HealthCheckPath": tg.pointer("/HealthCheckPath").and_then(|v| v.as_str()).unwrap_or("-"),
                    "HealthCheckProtocol": tg.pointer("/HealthCheckProtocol").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "target_groups": result }))
        }

        ("elbv2", "describe_target_health") => {
            let tg_arn = extract_param(params, "target_group_arn");
            if tg_arn.is_empty() {
                return Ok(json!({ "targets": [] }));
            }
            
            let xml = clients.http.query_request("elbv2", "DescribeTargetHealth", &[
                ("TargetGroupArn", &tg_arn)
            ]).await?;
            let json = xml_to_json(&xml)?;
            
            let targets_data = json.pointer("/DescribeTargetHealthResponse/DescribeTargetHealthResult/TargetHealthDescriptions/member");
            let target_list = match targets_data {
                Some(Value::Array(arr)) => arr.clone(),
                Some(obj @ Value::Object(_)) => vec![obj.clone()],
                _ => vec![],
            };
            
            let result: Vec<Value> = target_list.iter().map(|t| {
                json!({
                    "TargetId": t.pointer("/Target/Id").and_then(|v| v.as_str()).unwrap_or("-"),
                    "Port": t.pointer("/Target/Port").and_then(|v| v.as_str()).unwrap_or("-"),
                    "AvailabilityZone": t.pointer("/Target/AvailabilityZone").and_then(|v| v.as_str()).unwrap_or("-"),
                    "HealthState": t.pointer("/TargetHealth/State").and_then(|v| v.as_str()).unwrap_or("-"),
                    "HealthReason": t.pointer("/TargetHealth/Reason").and_then(|v| v.as_str()).unwrap_or("-"),
                    "HealthDescription": t.pointer("/TargetHealth/Description").and_then(|v| v.as_str()).unwrap_or("-"),
                })
            }).collect();
            
            Ok(json!({ "targets": result }))
        }

        // =====================================================================
        // Unknown operation - service not supported
        // =====================================================================
        _ => Err(anyhow!(
            "Unsupported operation: service='{}', method='{}'. Only 30 core AWS services are supported.",
            service,
            method
        )),
    }
}

// =============================================================================
// XML Parsing Helpers
// =============================================================================

/// Extract list from IAM response
fn extract_iam_list(json: &Value, list_key: &str, item_key: &str) -> Vec<Value> {
    // IAM structure: { "XXXResponse": { "XXXResult": { "ListKey": { "member": [...] } } } }
    let result = json.as_object()
        .and_then(|o| o.values().next())
        .and_then(|v| v.as_object())
        .and_then(|o| o.values().next())
        .and_then(|v| v.get(list_key))
        .and_then(|v| v.get(item_key));
    
    match result {
        Some(Value::Array(arr)) => arr.clone(),
        Some(obj @ Value::Object(_)) => vec![obj.clone()],
        _ => vec![],
    }
}

/// Extract list from EC2 response
fn extract_ec2_list(json: &Value, set_key: &str) -> Vec<Value> {
    // EC2 structure: { "XXXResponse": { "setKey": { "item": [...] } } }
    let items = json.as_object()
        .and_then(|o| o.values().next())
        .and_then(|v| v.get(set_key))
        .and_then(|v| v.get("item"));
    
    match items {
        Some(Value::Array(arr)) => arr.clone(),
        Some(obj @ Value::Object(_)) => vec![obj.clone()],
        _ => vec![],
    }
}

/// Extract list from RDS response
fn extract_rds_list(json: &Value, list_key: &str, item_key: &str) -> Vec<Value> {
    // RDS structure: { "XXXResponse": { "XXXResult": { "ListKey": { "ItemKey": [...] } } } }
    let result = json.as_object()
        .and_then(|o| o.values().next())
        .and_then(|v| v.as_object())
        .and_then(|o| o.values().next())
        .and_then(|v| v.get(list_key))
        .and_then(|v| v.get(item_key));
    
    match result {
        Some(Value::Array(arr)) => arr.clone(),
        Some(obj @ Value::Object(_)) => vec![obj.clone()],
        _ => vec![],
    }
}

/// Extract tags from EC2 resource
fn extract_tags(resource: &Value) -> Value {
    let mut tags = serde_json::Map::new();
    
    if let Some(tag_set) = resource.pointer("/tagSet/item") {
        let tag_list = match tag_set {
            Value::Array(arr) => arr.clone(),
            obj @ Value::Object(_) => vec![obj.clone()],
            _ => vec![],
        };
        
        for tag in tag_list {
            if let (Some(key), Some(value)) = (
                tag.pointer("/key").and_then(|v| v.as_str()),
                tag.pointer("/value").and_then(|v| v.as_str()),
            ) {
                tags.insert(key.to_string(), Value::String(value.to_string()));
            }
        }
    }
    
    Value::Object(tags)
}
