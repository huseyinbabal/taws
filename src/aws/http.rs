//! Lightweight AWS HTTP Client with SigV4 signing
//!
//! Replaces 55 AWS SDK crates with a single HTTP client

use anyhow::{anyhow, Result};
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4::SigningParams;
use aws_smithy_runtime_api::client::identity::Identity;
use reqwest::Client;
use std::collections::HashMap;
use std::time::SystemTime;
use tracing::{debug, trace, warn};

use super::credentials::Credentials;

const EUSC_PREFIX: &str = "eusc-";

/// Extract region from S3 URL patterns like:
/// - https://bucket.s3.us-west-1.amazonaws.com/
/// - https://bucket.s3-us-west-1.amazonaws.com/
fn extract_region_from_s3_url(url: &str) -> Option<String> {
    // Look for patterns like "s3.us-west-1.amazonaws.com" or "s3-us-west-1.amazonaws.com"
    let url_lower = url.to_lowercase();

    // Find "s3." or "s3-" followed by region
    for prefix in &["s3.", "s3-"] {
        if let Some(pos) = url_lower.find(prefix) {
            let after_prefix = &url_lower[pos + prefix.len()..];
            // Region format: xx-xxxx-N (e.g., us-west-1, eu-central-1, ap-southeast-2)
            if let Some(end) = after_prefix.find(".amazonaws.") {
                let region = &after_prefix[..end];
                // Validate it looks like a region (contains at least one hyphen and ends with digit)
                if region.contains('-')
                    && region
                        .chars()
                        .last()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                {
                    return Some(region.to_string());
                }
            }
        }
    }
    None
}

/// Mask sensitive credential values for logging
fn mask_credential(value: &str) -> String {
    if value.len() <= 8 {
        "*".repeat(value.len())
    } else {
        format!("{}...{}", &value[..4], &value[value.len() - 4..])
    }
}

/// AWS Service definition
#[derive(Debug, Clone)]
pub struct ServiceDefinition {
    /// Service signing name (e.g., "ec2", "sts", "elasticloadbalancing")
    pub signing_name: &'static str,
    /// Service endpoint prefix (e.g., "ec2", "sts", "elasticloadbalancing")
    pub endpoint_prefix: &'static str,
    /// API version (e.g., "2016-11-15" for EC2)
    pub api_version: &'static str,
    /// Protocol: "query", "json", "rest-json", "rest-xml"
    #[allow(dead_code)]
    pub protocol: Protocol,
    /// Target prefix for JSON protocol (e.g., "AWSCognitoIdentityProviderService")
    pub target_prefix: Option<&'static str>,
    /// Whether this is a global service (uses us-east-1)
    pub is_global: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Protocol {
    /// EC2/IAM style: Action=X&Version=Y as query params
    Query,
    /// JSON-RPC style with X-Amz-Target header
    Json,
    /// REST with JSON body
    RestJson,
    /// REST with XML body (S3)
    RestXml,
}

/// Service definitions for all 30 supported services
pub fn get_service(name: &str) -> Option<ServiceDefinition> {
    match name {
        "ec2" => Some(ServiceDefinition {
            signing_name: "ec2",
            endpoint_prefix: "ec2",
            api_version: "2016-11-15",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "s3" => Some(ServiceDefinition {
            signing_name: "s3",
            endpoint_prefix: "s3",
            api_version: "2006-03-01",
            protocol: Protocol::RestXml,
            target_prefix: None,
            is_global: false,
        }),
        "iam" => Some(ServiceDefinition {
            signing_name: "iam",
            endpoint_prefix: "iam",
            api_version: "2010-05-08",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: true,
        }),
        "lambda" => Some(ServiceDefinition {
            signing_name: "lambda",
            endpoint_prefix: "lambda",
            api_version: "2015-03-31",
            protocol: Protocol::RestJson,
            target_prefix: None,
            is_global: false,
        }),
        "rds" => Some(ServiceDefinition {
            signing_name: "rds",
            endpoint_prefix: "rds",
            api_version: "2014-10-31",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "dynamodb" => Some(ServiceDefinition {
            signing_name: "dynamodb",
            endpoint_prefix: "dynamodb",
            api_version: "2012-08-10",
            protocol: Protocol::Json,
            target_prefix: Some("DynamoDB_20120810"),
            is_global: false,
        }),
        "ecs" => Some(ServiceDefinition {
            signing_name: "ecs",
            endpoint_prefix: "ecs",
            api_version: "2014-11-13",
            protocol: Protocol::Json,
            target_prefix: Some("AmazonEC2ContainerServiceV20141113"),
            is_global: false,
        }),
        "eks" => Some(ServiceDefinition {
            signing_name: "eks",
            endpoint_prefix: "eks",
            api_version: "2017-11-01",
            protocol: Protocol::RestJson,
            target_prefix: None,
            is_global: false,
        }),
        "cloudformation" => Some(ServiceDefinition {
            signing_name: "cloudformation",
            endpoint_prefix: "cloudformation",
            api_version: "2010-05-15",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "cloudwatchlogs" | "logs" => Some(ServiceDefinition {
            signing_name: "logs",
            endpoint_prefix: "logs",
            api_version: "2014-03-28",
            protocol: Protocol::Json,
            target_prefix: Some("Logs_20140328"),
            is_global: false,
        }),
        "sqs" => Some(ServiceDefinition {
            signing_name: "sqs",
            endpoint_prefix: "sqs",
            api_version: "2012-11-05",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "sns" => Some(ServiceDefinition {
            signing_name: "sns",
            endpoint_prefix: "sns",
            api_version: "2010-03-31",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "secretsmanager" => Some(ServiceDefinition {
            signing_name: "secretsmanager",
            endpoint_prefix: "secretsmanager",
            api_version: "2017-10-17",
            protocol: Protocol::Json,
            target_prefix: Some("secretsmanager"),
            is_global: false,
        }),
        "ssm" => Some(ServiceDefinition {
            signing_name: "ssm",
            endpoint_prefix: "ssm",
            api_version: "2014-11-06",
            protocol: Protocol::Json,
            target_prefix: Some("AmazonSSM"),
            is_global: false,
        }),
        "route53" => Some(ServiceDefinition {
            signing_name: "route53",
            endpoint_prefix: "route53",
            api_version: "2013-04-01",
            protocol: Protocol::RestXml,
            target_prefix: None,
            is_global: true,
        }),
        "apigateway" => Some(ServiceDefinition {
            signing_name: "apigateway",
            endpoint_prefix: "apigateway",
            api_version: "2015-07-09",
            protocol: Protocol::RestJson,
            target_prefix: None,
            is_global: false,
        }),
        "sts" => Some(ServiceDefinition {
            signing_name: "sts",
            endpoint_prefix: "sts",
            api_version: "2011-06-15",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "ecr" => Some(ServiceDefinition {
            signing_name: "ecr",
            endpoint_prefix: "api.ecr",
            api_version: "2015-09-21",
            protocol: Protocol::Json,
            target_prefix: Some("AmazonEC2ContainerRegistry_V20150921"),
            is_global: false,
        }),
        "kms" => Some(ServiceDefinition {
            signing_name: "kms",
            endpoint_prefix: "kms",
            api_version: "2014-11-01",
            protocol: Protocol::Json,
            target_prefix: Some("TrentService"),
            is_global: false,
        }),
        "elasticache" => Some(ServiceDefinition {
            signing_name: "elasticache",
            endpoint_prefix: "elasticache",
            api_version: "2015-02-02",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "cloudfront" => Some(ServiceDefinition {
            signing_name: "cloudfront",
            endpoint_prefix: "cloudfront",
            api_version: "2020-05-31",
            protocol: Protocol::RestXml,
            target_prefix: None,
            is_global: true,
        }),
        "acm" => Some(ServiceDefinition {
            signing_name: "acm",
            endpoint_prefix: "acm",
            api_version: "2015-12-08",
            protocol: Protocol::Json,
            target_prefix: Some("CertificateManager"),
            is_global: false,
        }),
        "eventbridge" | "events" => Some(ServiceDefinition {
            signing_name: "events",
            endpoint_prefix: "events",
            api_version: "2015-10-07",
            protocol: Protocol::Json,
            target_prefix: Some("AWSEvents"),
            is_global: false,
        }),
        "codepipeline" => Some(ServiceDefinition {
            signing_name: "codepipeline",
            endpoint_prefix: "codepipeline",
            api_version: "2015-07-09",
            protocol: Protocol::Json,
            target_prefix: Some("CodePipeline_20150709"),
            is_global: false,
        }),
        "codebuild" => Some(ServiceDefinition {
            signing_name: "codebuild",
            endpoint_prefix: "codebuild",
            api_version: "2016-10-06",
            protocol: Protocol::Json,
            target_prefix: Some("CodeBuild_20161006"),
            is_global: false,
        }),
        "cognitoidentityprovider" | "cognito-idp" => Some(ServiceDefinition {
            signing_name: "cognito-idp",
            endpoint_prefix: "cognito-idp",
            api_version: "2016-04-18",
            protocol: Protocol::Json,
            target_prefix: Some("AWSCognitoIdentityProviderService"),
            is_global: false,
        }),
        "cloudtrail" => Some(ServiceDefinition {
            signing_name: "cloudtrail",
            endpoint_prefix: "cloudtrail",
            api_version: "2013-11-01",
            protocol: Protocol::Json,
            target_prefix: Some("com.amazonaws.cloudtrail.v20131101.CloudTrail_20131101"),
            is_global: false,
        }),
        "autoscaling" => Some(ServiceDefinition {
            signing_name: "autoscaling",
            endpoint_prefix: "autoscaling",
            api_version: "2011-01-01",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "elasticloadbalancing" | "elb" | "elbv2" => Some(ServiceDefinition {
            signing_name: "elasticloadbalancing",
            endpoint_prefix: "elasticloadbalancing",
            api_version: "2015-12-01",
            protocol: Protocol::Query,
            target_prefix: None,
            is_global: false,
        }),
        "athena" => Some(ServiceDefinition {
            signing_name: "athena",
            endpoint_prefix: "athena",
            api_version: "2017-05-18",
            protocol: Protocol::Json,
            target_prefix: Some("AmazonAthena"),
            is_global: false,
        }),
        _ => None,
    }
}

/// AWS HTTP Client
pub struct AwsHttpClient {
    http_client: Client,
    credentials: Credentials,
    region: String,
    endpoint_url: Option<String>,
}

impl AwsHttpClient {
    /// Create a new AWS HTTP client
    pub fn new(credentials: Credentials, region: &str, endpoint_url: Option<String>) -> Self {
        debug!(
            "Creating AWS HTTP client for region: {}, access_key: {}, endpoint_url: {:?}",
            region,
            mask_credential(&credentials.access_key_id),
            endpoint_url
        );
        Self {
            http_client: Client::new(),
            credentials,
            region: region.to_string(),
            endpoint_url,
        }
    }

    /// Update region
    pub fn set_region(&mut self, region: &str) {
        debug!("Switching region to: {}", region);
        self.region = region.to_string();
    }

    /// Update credentials
    pub fn set_credentials(&mut self, credentials: Credentials) {
        debug!(
            "Updating credentials, access_key: {}",
            mask_credential(&credentials.access_key_id)
        );
        self.credentials = credentials;
    }

    /// Determine which region should be used for a service (handles global services in ESC)
    fn effective_region<'a>(&'a self, service: &ServiceDefinition) -> &'a str {
        if service.is_global {
            if self.region.starts_with(EUSC_PREFIX) {
                &self.region
            } else {
                "us-east-1"
            }
        } else {
            &self.region
        }
    }

    /// Get the endpoint URL for a service, validating partition support when needed
    fn get_endpoint(&self, service: &ServiceDefinition) -> Result<String> {
        // If custom endpoint is set, use it for ALL services (LocalStack, etc.)
        if let Some(ref endpoint) = self.endpoint_url {
            return Ok(endpoint.clone());
        }

        let region = self.effective_region(service);
        let domain = if service.is_global {
            // Sovereign/global services should mirror the selected region's domain
            Self::endpoint_domain(&self.region)
        } else {
            Self::endpoint_domain(region)
        };

        // Special case for S3
        if service.signing_name == "s3" {
            return Ok(format!("https://s3.{}.{}", region, domain));
        }

        // Special case for global services
        if service.is_global {
            return match service.signing_name {
                "iam" => {
                    if domain == "amazonaws.eu" {
                        Ok(format!("https://iam.{}.{}", self.region, domain))
                    } else {
                        Ok("https://iam.amazonaws.com".to_string())
                    }
                }
                "cloudfront" if self.region.starts_with(EUSC_PREFIX) => Err(anyhow!(
                    "Service 'cloudfront' is not available yet in ESC regions"
                )),
                "cloudfront" => Ok(format!("https://cloudfront.{}", domain)),
                "route53" => Ok(format!("https://route53.{}", domain)),
                _ => Ok(format!("https://{}.{}", service.endpoint_prefix, domain)),
            };
        }

        Ok(format!(
            "https://{}.{}.{}",
            service.endpoint_prefix, region, domain
        ))
    }

    /// Determine the endpoint domain for a region (standard vs. sovereign)
    fn endpoint_domain(region: &str) -> &'static str {
        if region.starts_with(EUSC_PREFIX) {
            "amazonaws.eu"
        } else {
            "amazonaws.com"
        }
    }

    /// Make a Query protocol request (EC2, IAM, RDS, etc.)
    pub async fn query_request(
        &self,
        service_name: &str,
        action: &str,
        params: &[(&str, &str)],
    ) -> Result<String> {
        debug!("Query request: service={}, action={}", service_name, action);
        trace!("Query params: {:?}", params);

        let service = get_service(service_name)
            .ok_or_else(|| anyhow!("Unknown service: {}", service_name))?;

        let endpoint = self.get_endpoint(&service)?;
        debug!("Endpoint: {}", endpoint);

        // Build query string
        let mut query_params: Vec<(String, String)> = vec![
            ("Action".to_string(), action.to_string()),
            ("Version".to_string(), service.api_version.to_string()),
        ];
        for (k, v) in params {
            query_params.push((k.to_string(), v.to_string()));
        }

        let query_string: String = query_params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let url = format!("{}/?{}", endpoint, query_string);
        let body = "";

        self.signed_request(&service, "POST", &url, body, None)
            .await
    }

    /// Make a JSON protocol request (DynamoDB, ECS, Logs, etc.)
    pub async fn json_request(
        &self,
        service_name: &str,
        target: &str,
        body: &str,
    ) -> Result<String> {
        debug!("JSON request: service={}, target={}", service_name, target);
        trace!("JSON body: {}", body);

        let service = get_service(service_name)
            .ok_or_else(|| anyhow!("Unknown service: {}", service_name))?;

        let endpoint = self.get_endpoint(&service)?;
        let url = format!("{}/", endpoint);
        debug!("Endpoint: {}", endpoint);

        let target_header = format!(
            "{}.{}",
            service.target_prefix.unwrap_or(service.signing_name),
            target
        );

        let mut headers = HashMap::new();
        headers.insert("X-Amz-Target".to_string(), target_header);
        headers.insert(
            "Content-Type".to_string(),
            "application/x-amz-json-1.1".to_string(),
        );

        self.signed_request(&service, "POST", &url, body, Some(headers))
            .await
    }

    /// Make a REST-JSON request (Lambda, API Gateway, EKS, etc.)
    pub async fn rest_json_request(
        &self,
        service_name: &str,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<String> {
        debug!(
            "REST-JSON request: service={}, method={}, path={}",
            service_name, method, path
        );
        trace!("REST-JSON body: {:?}", body);

        let service = get_service(service_name)
            .ok_or_else(|| anyhow!("Unknown service: {}", service_name))?;

        let endpoint = self.get_endpoint(&service)?;
        let url = format!("{}{}", endpoint, path);
        debug!("URL: {}", url);

        let mut headers = HashMap::new();
        if body.is_some() {
            headers.insert("Content-Type".to_string(), "application/json".to_string());
        }

        self.signed_request(&service, method, &url, body.unwrap_or(""), Some(headers))
            .await
    }

    /// Make a REST-XML request (S3, Route53, CloudFront)
    pub async fn rest_xml_request(
        &self,
        service_name: &str,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<String> {
        debug!(
            "REST-XML request: service={}, method={}, path={}",
            service_name, method, path
        );

        let service = get_service(service_name)
            .ok_or_else(|| anyhow!("Unknown service: {}", service_name))?;

        let endpoint = self.get_endpoint(&service)?;
        let url = format!("{}{}", endpoint, path);
        debug!("URL: {}", url);

        self.signed_request(&service, method, &url, body.unwrap_or(""), None)
            .await
    }

    /// Make a REST-XML request to a specific S3 bucket region
    /// This is needed because S3 buckets exist in specific regions and
    /// requests must be sent to the correct regional endpoint
    pub async fn rest_xml_request_s3_bucket(
        &self,
        method: &str,
        bucket: &str,
        path: &str,
        body: Option<&str>,
        bucket_region: &str,
    ) -> Result<String> {
        debug!(
            "REST-XML S3 bucket request: bucket={}, region={}, method={}, path={}",
            bucket, bucket_region, method, path
        );

        let service = get_service("s3").ok_or_else(|| anyhow!("Unknown service: s3"))?;

        // Build S3 regional endpoint
        let domain = Self::endpoint_domain(bucket_region);
        let endpoint = format!("https://{}.s3.{}.{}", bucket, bucket_region, domain);
        let url = format!("{}{}", endpoint, path);
        debug!("URL: {}", url);

        self.signed_request_with_region(
            &service,
            method,
            &url,
            body.unwrap_or(""),
            None,
            bucket_region,
        )
        .await
    }

    /// Get the region for an S3 bucket using HEAD request to check x-amz-bucket-region header
    pub async fn get_bucket_region(&self, bucket: &str) -> Result<String> {
        debug!("Getting bucket region for: {}", bucket);

        // Use HEAD request to any S3 endpoint - AWS returns x-amz-bucket-region header
        // even for 301/400 responses, which tells us the correct region
        let mut domain_candidates = vec!["amazonaws.com"];
        if self.region.starts_with(EUSC_PREFIX) {
            domain_candidates.insert(0, "amazonaws.eu");
        }

        for domain in domain_candidates {
            let url = format!("https://{}.s3.{}/", bucket, domain);
            debug!("Probing bucket {} region via {}", bucket, url);

            let response = match self.http_client.head(&url).send().await {
                Ok(resp) => resp,
                Err(err) => {
                    debug!("Bucket region probe failed for {}: {}", url, err);
                    continue;
                }
            };

            // Check x-amz-bucket-region header (present in both success and redirect responses)
            if let Some(region) = response.headers().get("x-amz-bucket-region") {
                if let Ok(region_str) = region.to_str() {
                    debug!(
                        "Bucket {} is in region {} (from x-amz-bucket-region header)",
                        bucket, region_str
                    );
                    return Ok(region_str.to_string());
                }
            }

            // Fallback: if we got a 200, bucket is accessible from the probed region
            if response.status().is_success() {
                debug!(
                    "Bucket {} accessible via {} (HEAD succeeded)",
                    bucket, domain
                );
                return Ok(self.region.clone());
            }

            // If we got a redirect, try to parse the region from the Location header or body
            if response.status() == reqwest::StatusCode::MOVED_PERMANENTLY {
                // Check Location header for region hint
                if let Some(location) = response.headers().get("location") {
                    if let Ok(loc_str) = location.to_str() {
                        // Location might be like: https://bucket.s3.us-west-1.amazonaws.com/
                        // or https://bucket.s3-us-west-1.amazonaws.com/
                        if let Some(region) = extract_region_from_s3_url(loc_str) {
                            debug!(
                                "Bucket {} is in region {} (from Location header)",
                                bucket, region
                            );
                            return Ok(region);
                        }
                    }
                }
            }
        }

        // Default to currently selected region for sovereign clouds, otherwise us-east-1
        if self.region.starts_with(EUSC_PREFIX) {
            debug!(
                "Bucket {} defaulting to current region {}",
                bucket, self.region
            );
            Ok(self.region.clone())
        } else {
            debug!("Bucket {} defaulting to us-east-1", bucket);
            Ok("us-east-1".to_string())
        }
    }

    /// Make a signed request
    async fn signed_request(
        &self,
        service: &ServiceDefinition,
        method: &str,
        url: &str,
        body: &str,
        extra_headers: Option<HashMap<String, String>>,
    ) -> Result<String> {
        let region = self.effective_region(service);

        // Parse URL
        let parsed_url = url::Url::parse(url)?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow!("Invalid URL"))?;
        let path_and_query = if let Some(query) = parsed_url.query() {
            format!("{}?{}", parsed_url.path(), query)
        } else {
            parsed_url.path().to_string()
        };

        // Build headers
        let mut headers = vec![("host".to_string(), host.to_string())];

        if let Some(extra) = &extra_headers {
            for (k, v) in extra {
                headers.push((k.to_lowercase(), v.clone()));
            }
        }

        // Create identity for signing
        let creds = aws_credential_types::Credentials::new(
            &self.credentials.access_key_id,
            &self.credentials.secret_access_key,
            self.credentials.session_token.clone(),
            None,
            "taws",
        );
        let identity: Identity = creds.into();

        // Create signing params
        let signing_params = SigningParams::builder()
            .identity(&identity)
            .region(region)
            .name(service.signing_name)
            .time(SystemTime::now())
            .settings(SigningSettings::default())
            .build()?
            .into();

        // Create signable request
        // For S3, use UnsignedPayload for GET/DELETE requests without body
        let is_s3_unsigned = service.signing_name == "s3"
            && body.is_empty()
            && (method == "GET" || method == "DELETE");
        let signable_body = if is_s3_unsigned {
            SignableBody::UnsignedPayload
        } else if body.is_empty() {
            SignableBody::Bytes(&[])
        } else {
            SignableBody::Bytes(body.as_bytes())
        };

        // S3 requires x-amz-content-sha256 header
        if is_s3_unsigned {
            headers.push((
                "x-amz-content-sha256".to_string(),
                "UNSIGNED-PAYLOAD".to_string(),
            ));
        }

        let signable_request = SignableRequest::new(
            method,
            &path_and_query,
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            signable_body,
        )?;

        // Sign the request
        let (signing_instructions, _signature) =
            sign(signable_request, &signing_params)?.into_parts();

        // Build the actual request
        let mut request = match method {
            "GET" => self.http_client.get(url),
            "POST" => self.http_client.post(url),
            "PUT" => self.http_client.put(url),
            "DELETE" => self.http_client.delete(url),
            "PATCH" => self.http_client.patch(url),
            _ => return Err(anyhow!("Unsupported HTTP method: {}", method)),
        };

        // Apply signing headers
        for (name, value) in signing_instructions.headers() {
            request = request.header(name.to_string(), value.to_string());
        }

        // S3 requires x-amz-content-sha256 header explicitly
        if is_s3_unsigned {
            request = request.header("x-amz-content-sha256", "UNSIGNED-PAYLOAD");
        }

        // Apply extra headers
        if let Some(extra) = extra_headers {
            for (k, v) in extra {
                request = request.header(&k, &v);
            }
        }

        // Set body if present
        if !body.is_empty() {
            request = request.body(body.to_string());
        }

        // Send request
        trace!("Sending {} request to {}", method, url);
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        debug!("Response status: {}", status);
        trace!(
            "Response body (first 2000 chars): {}",
            &text[..text.len().min(2000)]
        );

        if !status.is_success() {
            warn!(
                "AWS request failed: status={}, body={}",
                status,
                &text[..text.len().min(500)]
            );
            return Err(anyhow!("AWS request failed ({}): {}", status, text));
        }

        Ok(text)
    }

    /// Make a signed request with explicit region override
    /// Used for S3 bucket operations where the bucket may be in a different region
    async fn signed_request_with_region(
        &self,
        service: &ServiceDefinition,
        method: &str,
        url: &str,
        body: &str,
        extra_headers: Option<HashMap<String, String>>,
        region: &str,
    ) -> Result<String> {
        // Parse URL
        let parsed_url = url::Url::parse(url)?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow!("Invalid URL"))?;
        let path_and_query = if let Some(query) = parsed_url.query() {
            format!("{}?{}", parsed_url.path(), query)
        } else {
            parsed_url.path().to_string()
        };

        // Build headers
        let mut headers = vec![("host".to_string(), host.to_string())];

        if let Some(extra) = &extra_headers {
            for (k, v) in extra {
                headers.push((k.to_lowercase(), v.clone()));
            }
        }

        // Create identity for signing
        let creds = aws_credential_types::Credentials::new(
            &self.credentials.access_key_id,
            &self.credentials.secret_access_key,
            self.credentials.session_token.clone(),
            None,
            "taws",
        );
        let identity: Identity = creds.into();

        // Create signing params with explicit region
        let signing_params = SigningParams::builder()
            .identity(&identity)
            .region(region)
            .name(service.signing_name)
            .time(SystemTime::now())
            .settings(SigningSettings::default())
            .build()?
            .into();

        // Create signable request
        let is_s3_unsigned = service.signing_name == "s3"
            && body.is_empty()
            && (method == "GET" || method == "DELETE");
        let signable_body = if is_s3_unsigned {
            SignableBody::UnsignedPayload
        } else if body.is_empty() {
            SignableBody::Bytes(&[])
        } else {
            SignableBody::Bytes(body.as_bytes())
        };

        if is_s3_unsigned {
            headers.push((
                "x-amz-content-sha256".to_string(),
                "UNSIGNED-PAYLOAD".to_string(),
            ));
        }

        let signable_request = SignableRequest::new(
            method,
            &path_and_query,
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            signable_body,
        )?;

        // Sign the request
        let (signing_instructions, _signature) =
            sign(signable_request, &signing_params)?.into_parts();

        // Build the actual request
        let mut request = match method {
            "GET" => self.http_client.get(url),
            "POST" => self.http_client.post(url),
            "PUT" => self.http_client.put(url),
            "DELETE" => self.http_client.delete(url),
            "PATCH" => self.http_client.patch(url),
            _ => return Err(anyhow!("Unsupported HTTP method: {}", method)),
        };

        // Apply signing headers
        for (name, value) in signing_instructions.headers() {
            request = request.header(name.to_string(), value.to_string());
        }

        if is_s3_unsigned {
            request = request.header("x-amz-content-sha256", "UNSIGNED-PAYLOAD");
        }

        // Apply extra headers
        if let Some(extra) = extra_headers {
            for (k, v) in extra {
                request = request.header(&k, &v);
            }
        }

        // Set body if present
        if !body.is_empty() {
            request = request.body(body.to_string());
        }

        // Send request
        trace!("Sending {} request to {} (region: {})", method, url, region);
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        debug!("Response status: {}", status);
        trace!(
            "Response body (first 2000 chars): {}",
            &text[..text.len().min(2000)]
        );

        if !status.is_success() {
            warn!(
                "AWS request failed: status={}, body={}",
                status,
                &text[..text.len().min(500)]
            );
            return Err(anyhow!("AWS request failed ({}): {}", status, text));
        }

        Ok(text)
    }
}

/// Parse XML response to JSON using quick-xml
pub fn xml_to_json(xml: &str) -> Result<serde_json::Value> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use serde_json::{Map, Value};
    fn parse_element(reader: &mut Reader<&[u8]>) -> Result<Value> {
        let mut map: Map<String, Value> = Map::new();
        let mut buf = Vec::new();
        let mut current_text = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let child_value = parse_element(reader)?;

                    // Handle duplicate keys by converting to array
                    if let Some(existing) = map.get_mut(&tag_name) {
                        match existing {
                            Value::Array(arr) => arr.push(child_value),
                            _ => {
                                let old = existing.take();
                                *existing = Value::Array(vec![old, child_value]);
                            }
                        }
                    } else {
                        map.insert(tag_name, child_value);
                    }
                }
                Ok(Event::Text(e)) => {
                    let text = e.xml_content().unwrap_or_default().trim().to_string();
                    if !text.is_empty() {
                        current_text = text;
                    }
                }
                Ok(Event::End(_)) => {
                    break;
                }
                Ok(Event::Empty(e)) => {
                    let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    map.insert(tag_name, Value::Null);
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(anyhow!("XML parse error: {}", e)),
                _ => {}
            }
            buf.clear();
        }

        // If we only collected text and no child elements, return the text
        if map.is_empty() && !current_text.is_empty() {
            Ok(Value::String(current_text))
        } else {
            Ok(Value::Object(map))
        }
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut root_map: Map<String, Value> = Map::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let child_value = parse_element(&mut reader)?;
                root_map.insert(tag_name, child_value);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(anyhow!("XML parse error: {}", e)),
        }
        buf.clear();
    }

    Ok(Value::Object(root_map))
}

#[cfg(test)]
mod tests {
    use super::{get_service, AwsHttpClient, Credentials};

    fn dummy_credentials() -> Credentials {
        Credentials {
            access_key_id: "TESTACCESSKEY".to_string(),
            secret_access_key: "TESTSECRETKEY".to_string(),
            session_token: None,
        }
    }

    fn client_with_region(region: &str) -> AwsHttpClient {
        AwsHttpClient::new(dummy_credentials(), region, None)
    }

    #[test]
    fn route53_uses_partition_domain_in_esc() {
        let client = client_with_region("eusc-de-east-1");
        let service = get_service("route53").expect("route53 service definition");
        let endpoint = client.get_endpoint(&service).expect("route53 endpoint");
        assert_eq!(endpoint, "https://route53.amazonaws.eu");
    }

    #[test]
    fn route53_uses_standard_domain_elsewhere() {
        let client = client_with_region("us-west-2");
        let service = get_service("route53").expect("route53 service definition");
        let endpoint = client.get_endpoint(&service).expect("route53 endpoint");
        assert_eq!(endpoint, "https://route53.amazonaws.com");
    }

    #[test]
    fn iam_includes_region_for_esc() {
        let client = client_with_region("eusc-de-east-1");
        let service = get_service("iam").expect("iam service definition");
        let endpoint = client.get_endpoint(&service).expect("iam endpoint");
        assert_eq!(endpoint, "https://iam.eusc-de-east-1.amazonaws.eu");
    }

    #[test]
    fn iam_uses_classic_endpoint_for_standard_regions() {
        let client = client_with_region("us-west-2");
        let service = get_service("iam").expect("iam service definition");
        let endpoint = client.get_endpoint(&service).expect("iam endpoint");
        assert_eq!(endpoint, "https://iam.amazonaws.com");
    }

    #[test]
    fn cloudfront_returns_not_available_error_in_esc() {
        let client = client_with_region("eusc-de-east-1");
        let service = get_service("cloudfront").expect("cloudfront service definition");
        let err = client
            .get_endpoint(&service)
            .expect_err("cloudfront should error in esc");
        assert_eq!(
            err.to_string(),
            "Service 'cloudfront' is not available in ESC regions yet"
        );
    }

    #[test]
    fn cloudfront_works_in_standard_regions() {
        let client = client_with_region("us-east-1");
        let service = get_service("cloudfront").expect("cloudfront service definition");
        let endpoint = client.get_endpoint(&service).expect("cloudfront endpoint");
        assert_eq!(endpoint, "https://cloudfront.amazonaws.com");
    }
}
