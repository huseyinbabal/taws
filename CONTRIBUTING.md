# Contributing to taws

Thank you for your interest in contributing to taws! This document provides guidelines and information for contributors.

## Before You Start

**Important:** Before adding a new AWS service or major feature, please start a discussion in our [GitHub Discussions](https://github.com/huseyinbabal/taws/discussions) board. This helps us:

- Avoid duplicate work
- Discuss the best approach
- Ensure the feature aligns with project goals
- Get community feedback

## How to Contribute

1. **Fork the repository**
2. **Create your feature branch** (`git checkout -b feature/amazing-feature`)
3. **Commit your changes** (`git commit -m 'Add some amazing feature'`)
4. **Push to the branch** (`git push origin feature/amazing-feature`)
5. **Open a Pull Request**

## Development Setup

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/taws.git
cd taws

# Build the project
cargo build

# Run in development mode
cargo run

# Run tests
cargo test

# Check formatting
cargo fmt --check

# Run linter
cargo clippy
```

## Architecture

taws follows a **fully data-driven architecture** where AWS resource definitions, API configurations, actions, and field mappings are all stored in JSON configuration files. This makes it easy to add new resource types without writing any Rust code.

```
src/
├── resources/              # JSON resource definitions (one per service)
│   ├── ec2.json
│   ├── lambda.json
│   ├── s3.json
│   └── ...
├── resource/
│   ├── registry.rs         # Resource registry and JSON loading
│   ├── fetcher.rs          # Generic resource fetcher
│   ├── dispatch.rs         # Unified API dispatcher
│   ├── protocol.rs         # API protocol types and configs
│   ├── field_mapper.rs     # Field transformation (tags, bytes, etc.)
│   ├── path_extractor.rs   # JSON path extraction utilities
│   └── handlers/           # Protocol-specific handlers
│       ├── query.rs        # EC2/IAM style (XML response)
│       ├── json.rs         # JSON-RPC style (DynamoDB, ECS)
│       ├── rest_json.rs    # REST + JSON (Lambda, EKS)
│       └── rest_xml.rs     # REST + XML (S3, Route53)
├── aws/
│   ├── client.rs           # AWS HTTP client management
│   ├── credentials.rs      # Credential loading (profiles, env vars)
│   ├── http.rs             # Lightweight HTTP client with SigV4 signing
│   └── profiles.rs         # AWS profile handling
└── ui/
    ├── table.rs            # Resource table view
    ├── details.rs          # Resource details view
    └── ...
```

### Lightweight Design

taws uses a custom lightweight HTTP client with AWS SigV4 signing instead of the full AWS SDK. This results in:

- **Fast builds** - ~100 dependencies vs ~500+ with full SDK
- **Small binary** - ~5MB release binary
- **Quick compilation** - Seconds instead of minutes

## Adding a New AWS Service

Adding a new AWS service is now **completely data-driven** - you only need to edit JSON files, no Rust code required!

### 1. Start a Discussion

Before writing any code, [open a discussion](https://github.com/huseyinbabal/taws/discussions/new?category=ideas) to propose the new service. Include:

- Which AWS service you want to add
- Which resources/operations you plan to support
- Why this service would be valuable

### 2. Add the Service Definition

Add the AWS service definition to `src/aws/http.rs`:

```rust
"myservice" => Some(ServiceDefinition {
    signing_name: "myservice",
    endpoint_prefix: "myservice",
    api_version: "2023-01-01",
    protocol: Protocol::Json,  // or Query, RestJson, RestXml
    target_prefix: Some("MyService"),  // for JSON protocol
    is_global: false,
}),
```

### 3. Add Resource JSON Definition

Create `src/resources/myservice.json`. The JSON file contains everything needed - no Rust code required:

```json
{
  "resources": {
    "myservice-items": {
      "display_name": "MyService Items",
      "service": "myservice",
      "sdk_method": "list_items",
      "response_path": "items",
      "id_field": "ItemId",
      "name_field": "ItemName",
      "is_global": false,
      "columns": [
        { "header": "ID", "json_path": "ItemId", "width": 20 },
        { "header": "NAME", "json_path": "ItemName", "width": 30 },
        { "header": "STATUS", "json_path": "Status", "width": 15, "color_map": "state" }
      ],
      "actions": [
        { "key": "ctrl+d", "display_name": "Delete", "shortcut": "ctrl+d", "sdk_method": "delete_item", 
          "confirm": { "message": "Delete item", "default_yes": false, "destructive": true } }
      ],
      "api_config": {
        "protocol": "json",
        "action": "ListItems",
        "response_root": "/Items"
      },
      "field_mappings": {
        "ItemId": { "source": "/ItemId", "default": "-" },
        "ItemName": { "source": "/ItemName", "default": "-" },
        "Status": { "source": "/Status", "default": "-" }
      },
      "action_configs": {
        "delete_item": {
          "action_id": "delete_item",
          "protocol": "json",
          "action": "DeleteItem",
          "body_template": "{\"ItemId\": \"{resource_id}\"}"
        }
      },
      "describe_config": {
        "protocol": "json",
        "action": "DescribeItem",
        "body_template": "{\"ItemId\": \"{resource_id}\"}"
      }
    }
  }
}
```

### JSON Configuration Reference

#### Protocol Types

| Protocol | Description | Examples |
|----------|-------------|----------|
| `query` | Query params + XML response | EC2, IAM, RDS, ELBv2 |
| `json` | JSON-RPC with X-Amz-Target header | DynamoDB, ECS, SecretsManager |
| `rest-json` | REST API with JSON body | Lambda, EKS, API Gateway |
| `rest-xml` | REST API with XML body | S3, Route53, CloudFront |

#### api_config Fields

| Field | Description |
|-------|-------------|
| `protocol` | One of: `query`, `json`, `rest-json`, `rest-xml` |
| `action` | API action name (e.g., `ListItems`, `DescribeInstances`) |
| `method` | HTTP method for REST protocols (default: `GET`) |
| `path` | URL path for REST protocols (e.g., `/2015-03-31/functions`) |
| `response_root` | JSON pointer to extract items from response |
| `pagination` | Pagination config (input_token, output_token, etc.) |

#### field_mappings Fields

| Field | Description |
|-------|-------------|
| `source` | JSON pointer path to extract value (e.g., `/instanceId`) |
| `default` | Default value if path not found |
| `transform` | Optional transform: `tags_to_map`, `format_bytes`, `bool_to_yes_no`, `array_to_csv` |

#### action_configs Fields

| Field | Description |
|-------|-------------|
| `action_id` | Unique action identifier |
| `protocol` | API protocol to use |
| `action` | API action name |
| `id_param` | Parameter name for resource ID |
| `body_template` | JSON body template with `{resource_id}` placeholder |
| `static_params` | Static parameters to include |

### 4. Test Your Changes

```bash
# Build and run
cargo build && cargo run

# Test the new resource
# Press : and type your resource name

# Run tests and linter
cargo test
cargo clippy -- -D warnings
```

## Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Pass all clippy lints (`cargo clippy`)
- Write descriptive commit messages
- Add comments for complex logic

## Pull Request Guidelines

- Keep PRs focused on a single feature or fix
- Update documentation if needed
- Ensure all tests pass
- Reference any related issues or discussions

## Questions?

If you have questions, feel free to:

- Open a [Discussion](https://github.com/huseyinbabal/taws/discussions)
- Check existing issues and PRs

Thank you for contributing!
