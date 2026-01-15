mod fetcher;
mod registry;

// Data-driven dispatch infrastructure
pub mod dispatch;
pub mod field_mapper;
pub mod handlers;
pub mod path_extractor;
pub mod protocol;

pub use dispatch::{describe_resource, execute_action, format_log_timestamp, invoke_sdk};
pub use fetcher::{extract_json_value, fetch_resources, fetch_resources_paginated, ResourceFilter};
pub use registry::*;
