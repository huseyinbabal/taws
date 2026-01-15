//! SDK Dispatcher - AWS API calls using lightweight HTTP client
//!
//! This module handles AWS API calls that haven't been migrated to data-driven dispatch.
//! Most list operations now use the data-driven system in data_driven_dispatch.rs.
//! This file contains:
//! - Action handlers (write operations like start/stop/delete)
//! - Describe handlers (single resource details)
//! - Legacy list operations for special cases (S3 objects, STS)

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
            clients
                .http
                .query_request("ec2", "StartInstances", &[("InstanceId.1", resource_id)])
                .await?;
            Ok(())
        }
        ("ec2", "stop_instance") => {
            clients
                .http
                .query_request("ec2", "StopInstances", &[("InstanceId.1", resource_id)])
                .await?;
            Ok(())
        }
        ("ec2", "reboot_instance") => {
            clients
                .http
                .query_request("ec2", "RebootInstances", &[("InstanceId.1", resource_id)])
                .await?;
            Ok(())
        }
        ("ec2", "terminate_instance") => {
            clients
                .http
                .query_request(
                    "ec2",
                    "TerminateInstances",
                    &[("InstanceId.1", resource_id)],
                )
                .await?;
            Ok(())
        }

        // Lambda Actions
        ("lambda", "invoke_function") => {
            clients
                .http
                .rest_json_request(
                    "lambda",
                    "POST",
                    &format!("/2015-03-31/functions/{}/invocations", resource_id),
                    Some("{}"),
                )
                .await?;
            Ok(())
        }
        ("lambda", "delete_function") => {
            clients
                .http
                .rest_json_request(
                    "lambda",
                    "DELETE",
                    &format!("/2015-03-31/functions/{}", resource_id),
                    None,
                )
                .await?;
            Ok(())
        }

        // RDS Actions
        ("rds", "start_db_instance") => {
            clients
                .http
                .query_request(
                    "rds",
                    "StartDBInstance",
                    &[("DBInstanceIdentifier", resource_id)],
                )
                .await?;
            Ok(())
        }
        ("rds", "stop_db_instance") => {
            clients
                .http
                .query_request(
                    "rds",
                    "StopDBInstance",
                    &[("DBInstanceIdentifier", resource_id)],
                )
                .await?;
            Ok(())
        }
        ("rds", "reboot_db_instance") => {
            clients
                .http
                .query_request(
                    "rds",
                    "RebootDBInstance",
                    &[("DBInstanceIdentifier", resource_id)],
                )
                .await?;
            Ok(())
        }
        ("rds", "delete_db_instance") => {
            clients
                .http
                .query_request(
                    "rds",
                    "DeleteDBInstance",
                    &[
                        ("DBInstanceIdentifier", resource_id),
                        ("SkipFinalSnapshot", "true"),
                    ],
                )
                .await?;
            Ok(())
        }
        ("rds", "delete_db_snapshot") => {
            clients
                .http
                .query_request(
                    "rds",
                    "DeleteDBSnapshot",
                    &[("DBSnapshotIdentifier", resource_id)],
                )
                .await?;
            Ok(())
        }

        // S3 Actions
        ("s3", "delete_bucket") => {
            clients
                .http
                .rest_xml_request("s3", "DELETE", &format!("/{}", resource_id), None)
                .await?;
            Ok(())
        }

        // SQS Actions
        ("sqs", "purge_queue") => {
            clients
                .http
                .query_request("sqs", "PurgeQueue", &[("QueueUrl", resource_id)])
                .await?;
            Ok(())
        }
        ("sqs", "delete_queue") => {
            clients
                .http
                .query_request("sqs", "DeleteQueue", &[("QueueUrl", resource_id)])
                .await?;
            Ok(())
        }

        // SNS Actions
        ("sns", "delete_topic") => {
            clients
                .http
                .query_request("sns", "DeleteTopic", &[("TopicArn", resource_id)])
                .await?;
            Ok(())
        }

        // CloudFormation Actions
        ("cloudformation", "delete_stack") => {
            clients
                .http
                .query_request(
                    "cloudformation",
                    "DeleteStack",
                    &[("StackName", resource_id)],
                )
                .await?;
            Ok(())
        }

        // ECS Actions
        ("ecs", "delete_cluster") => {
            clients
                .http
                .json_request(
                    "ecs",
                    "DeleteCluster",
                    &json!({ "cluster": resource_id }).to_string(),
                )
                .await?;
            Ok(())
        }
        ("ecs", "delete_service") => {
            // Service ARN format: arn:aws:ecs:region:account:service/cluster/service-name
            // Extract cluster from ARN
            let parts: Vec<&str> = resource_id.split('/').collect();
            let cluster = if parts.len() >= 2 {
                parts[parts.len() - 2]
            } else {
                return Err(anyhow!("Invalid service ARN format"));
            };

            clients
                .http
                .json_request(
                    "ecs",
                    "DeleteService",
                    &json!({
                        "cluster": cluster,
                        "service": resource_id,
                        "force": true
                    })
                    .to_string(),
                )
                .await?;
            Ok(())
        }
        ("ecs", "stop_task") => {
            // Task ARN format: arn:aws:ecs:region:account:task/cluster/task-id
            let parts: Vec<&str> = resource_id.split('/').collect();
            let cluster = if parts.len() >= 2 {
                parts[parts.len() - 2]
            } else {
                return Err(anyhow!("Invalid task ARN format"));
            };

            clients
                .http
                .json_request(
                    "ecs",
                    "StopTask",
                    &json!({
                        "cluster": cluster,
                        "task": resource_id
                    })
                    .to_string(),
                )
                .await?;
            Ok(())
        }

        // Auto Scaling Actions
        ("autoscaling", "delete_auto_scaling_group") => {
            clients
                .http
                .query_request(
                    "autoscaling",
                    "DeleteAutoScalingGroup",
                    &[
                        ("AutoScalingGroupName", resource_id),
                        ("ForceDelete", "true"),
                    ],
                )
                .await?;
            Ok(())
        }

        // ELBv2 Actions
        ("elbv2", "delete_load_balancer") => {
            clients
                .http
                .query_request(
                    "elbv2",
                    "DeleteLoadBalancer",
                    &[("LoadBalancerArn", resource_id)],
                )
                .await?;
            Ok(())
        }
        ("elbv2", "delete_listener") => {
            clients
                .http
                .query_request("elbv2", "DeleteListener", &[("ListenerArn", resource_id)])
                .await?;
            Ok(())
        }
        ("elbv2", "delete_rule") => {
            clients
                .http
                .query_request("elbv2", "DeleteRule", &[("RuleArn", resource_id)])
                .await?;
            Ok(())
        }
        ("elbv2", "delete_target_group") => {
            clients
                .http
                .query_request(
                    "elbv2",
                    "DeleteTargetGroup",
                    &[("TargetGroupArn", resource_id)],
                )
                .await?;
            Ok(())
        }
        ("elbv2", "deregister_targets") => {
            // resource_id format: target_group_arn|target_id
            let parts: Vec<&str> = resource_id.split('|').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "Invalid target format, expected target_group_arn|target_id"
                ));
            }
            clients
                .http
                .query_request(
                    "elbv2",
                    "DeregisterTargets",
                    &[
                        ("TargetGroupArn", parts[0]),
                        ("Targets.member.1.Id", parts[1]),
                    ],
                )
                .await?;
            Ok(())
        }

        // EKS Actions
        ("eks", "delete_cluster") => {
            clients
                .http
                .rest_json_request("eks", "DELETE", &format!("/clusters/{}", resource_id), None)
                .await?;
            Ok(())
        }

        // Secrets Manager Actions
        ("secretsmanager", "delete_secret") => {
            clients
                .http
                .json_request(
                    "secretsmanager",
                    "DeleteSecret",
                    &json!({
                        "SecretId": resource_id,
                        "ForceDeleteWithoutRecovery": true
                    })
                    .to_string(),
                )
                .await?;
            Ok(())
        }

        // Unknown action
        _ => Err(anyhow!(
            "Unknown action: service='{}', action='{}'",
            service,
            action
        )),
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
    debug!(
        "Describing resource: {} with id: {}",
        resource_key, resource_id
    );

    match resource_key {
        "ec2-instances" => {
            let xml = clients
                .http
                .query_request("ec2", "DescribeInstances", &[("InstanceId.1", resource_id)])
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(reservations) =
                json.pointer("/DescribeInstancesResponse/reservationSet/item")
            {
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
            let mut result = json!({
                "BucketName": resource_id,
            });

            let bucket_region = clients
                .http
                .get_bucket_region(resource_id)
                .await
                .unwrap_or_else(|_| "us-east-1".to_string());
            result["Region"] = json!(&bucket_region);

            if let Ok(xml) = clients
                .http
                .rest_xml_request_s3_bucket("GET", resource_id, "?versioning", None, &bucket_region)
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

            if let Ok(xml) = clients
                .http
                .rest_xml_request_s3_bucket("GET", resource_id, "?encryption", None, &bucket_region)
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

        "lambda-functions" => {
            let response = clients
                .http
                .rest_json_request(
                    "lambda",
                    "GET",
                    &format!("/2015-03-31/functions/{}", resource_id),
                    None,
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json)
        }

        "rds-instances" => {
            let xml = clients
                .http
                .query_request(
                    "rds",
                    "DescribeDBInstances",
                    &[("DBInstanceIdentifier", resource_id)],
                )
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(instances) = json.pointer(
                "/DescribeDBInstancesResponse/DescribeDBInstancesResult/DBInstances/DBInstance",
            ) {
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
            let xml = clients
                .http
                .query_request("iam", "GetUser", &[("UserName", resource_id)])
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(user) = json.pointer("/GetUserResponse/GetUserResult/User") {
                return Ok(user.clone());
            }
            Err(anyhow!("IAM user not found"))
        }

        "iam-roles" => {
            let xml = clients
                .http
                .query_request("iam", "GetRole", &[("RoleName", resource_id)])
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(role) = json.pointer("/GetRoleResponse/GetRoleResult/Role") {
                return Ok(role.clone());
            }
            Err(anyhow!("IAM role not found"))
        }

        "dynamodb-tables" => {
            let response = clients
                .http
                .json_request(
                    "dynamodb",
                    "DescribeTable",
                    &json!({ "TableName": resource_id }).to_string(),
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json.get("Table").cloned().unwrap_or(json))
        }

        "eks-clusters" => {
            let response = clients
                .http
                .rest_json_request("eks", "GET", &format!("/clusters/{}", resource_id), None)
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json.get("cluster").cloned().unwrap_or(json))
        }

        "ecs-clusters" => {
            let response = clients
                .http
                .json_request(
                    "ecs",
                    "DescribeClusters",
                    &json!({ "clusters": [resource_id] }).to_string(),
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            if let Some(clusters) = json.get("clusters").and_then(|c| c.as_array()) {
                if let Some(cluster) = clusters.first() {
                    return Ok(cluster.clone());
                }
            }
            Err(anyhow!("ECS cluster not found"))
        }

        "secretsmanager-secrets" => {
            let response = clients
                .http
                .json_request(
                    "secretsmanager",
                    "DescribeSecret",
                    &json!({ "SecretId": resource_id }).to_string(),
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json)
        }

        "kms-keys" => {
            let response = clients
                .http
                .json_request(
                    "kms",
                    "DescribeKey",
                    &json!({ "KeyId": resource_id }).to_string(),
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json.get("KeyMetadata").cloned().unwrap_or(json))
        }

        "elbv2-load-balancers" => {
            let xml = clients
                .http
                .query_request(
                    "elbv2",
                    "DescribeLoadBalancers",
                    &[("LoadBalancerArns.member.1", resource_id)],
                )
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(lbs) = json.pointer(
                "/DescribeLoadBalancersResponse/DescribeLoadBalancersResult/LoadBalancers/member",
            ) {
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
            let xml = clients
                .http
                .query_request(
                    "elbv2",
                    "DescribeTargetGroups",
                    &[("TargetGroupArns.member.1", resource_id)],
                )
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(tgs) = json.pointer(
                "/DescribeTargetGroupsResponse/DescribeTargetGroupsResult/TargetGroups/member",
            ) {
                let tg = match tgs {
                    Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
                    obj @ Value::Object(_) => obj.clone(),
                    _ => Value::Null,
                };
                return Ok(tg);
            }
            Err(anyhow!("Target group not found"))
        }

        _ => {
            debug!(
                "No describe implementation for {}, falling back to list data",
                resource_key
            );
            Err(anyhow!("Describe not implemented for {}", resource_key))
        }
    }
}

// =============================================================================
// Legacy List Operations (special cases only)
// =============================================================================

/// Invoke an AWS API method and return the response as JSON.
/// Most list operations now use data-driven dispatch. This function only handles
/// special cases that can't be easily configured (S3 objects, STS).
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
                    let prefix_val = p
                        .pointer("/Prefix")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-");
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
                    let size = obj
                        .pointer("/Size")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0");
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

        // CloudWatch Logs - tail_logs (streaming operation, not a simple list)
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

        // Unknown operation - this shouldn't happen if data-driven dispatch is working
        _ => Err(anyhow!(
            "Operation not handled by legacy dispatch: service='{}', method='{}'. This should use data-driven dispatch.",
            service,
            method
        )),
    }
}
