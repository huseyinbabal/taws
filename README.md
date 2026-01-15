<p align="center">
  <img src="assets/taws-logo.png" alt="taws" width="400"/>
</p>

# taws - Terminal UI for AWS

**taws** provides a terminal UI to interact with your AWS resources. The aim of this project is to make it easier to navigate, observe, and manage your AWS infrastructure in the wild.

---

[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

---

## Screenshots

<p align="center">
  <img src="assets/screenshot-ec2.png" alt="EC2 Instances View" width="800"/>
</p>

<p align="center">
  <img src="assets/screenshot-lambda.png" alt="Lambda Functions View" width="800"/>
</p>

---

## Features

- **Multi-Profile Support** - Easily switch between AWS profiles
- **Multi-Region Support** - Navigate across different AWS regions
- **94+ Resource Types** - Browse and manage resources across 60+ AWS services
- **Manual Refresh** - Refresh resources with a single keystroke
- **Pagination** - Navigate through large resource lists with `]` / `[` keys
- **Keyboard-Driven** - Vim-like navigation and commands
- **Resource Actions** - Start, stop, terminate EC2 instances directly
- **Detailed Views** - JSON/YAML view of resource details
- **Filtering** - Filter resources by name or attributes
- **Autocomplete** - Smart resource type autocomplete with fuzzy matching

---

## Installation

### Homebrew (macOS/Linux)

```bash
brew install huseyinbabal/tap/taws
```

### Scoop (Windows)

```powershell
scoop bucket add huseyinbabal https://github.com/huseyinbabal/scoop-bucket
scoop install taws
```

### Download Pre-built Binaries

Download the latest release from the [Releases page](https://github.com/huseyinbabal/taws/releases/latest).

| Platform | Architecture | Download |
|----------|--------------|----------|
| **macOS** | Apple Silicon (M1/M2/M3) | `taws-aarch64-apple-darwin.tar.gz` |
| **macOS** | Intel | `taws-x86_64-apple-darwin.tar.gz` |
| **Linux** | x86_64 (musl) | `taws-x86_64-unknown-linux-musl.tar.gz` |
| **Linux** | ARM64 (musl) | `taws-aarch64-unknown-linux-musl.tar.gz` |
| **Windows** | x86_64 | `taws-x86_64-pc-windows-msvc.zip` |

#### Quick Install (macOS/Linux)

```bash
# macOS Apple Silicon
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-aarch64-apple-darwin.tar.gz | tar xz
sudo mv taws /usr/local/bin/

# macOS Intel
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-x86_64-apple-darwin.tar.gz | tar xz
sudo mv taws /usr/local/bin/

# Linux x86_64 (musl - works on Alpine, Void, etc.)
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-x86_64-unknown-linux-musl.tar.gz | tar xz
sudo mv taws /usr/local/bin/

# Linux ARM64 (musl - works on Alpine, Void, etc.)
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-aarch64-unknown-linux-musl.tar.gz | tar xz
sudo mv taws /usr/local/bin/
```

#### Windows

1. Download `taws-x86_64-pc-windows-msvc.zip` from the [Releases page](https://github.com/huseyinbabal/taws/releases/latest)
2. Extract the zip file
3. Add the extracted folder to your PATH, or move `taws.exe` to a directory in your PATH

### Using Cargo

```bash
cargo install taws
```

### Using Docker

```bash
# Run interactively
docker run --rm -it ghcr.io/huseyinbabal/taws

# Launch with a specific profile (mount AWS credentials)
docker run --rm -it \
  -v ~/.aws:/root/.aws:ro \
  ghcr.io/huseyinbabal/taws --profile production

# Launch in a specific region
docker run --rm -it \
  -v ~/.aws:/root/.aws:ro \
  ghcr.io/huseyinbabal/taws --region us-west-2

# Using environment variables
docker run --rm -it \
  -e AWS_ACCESS_KEY_ID=$AWS_ACCESS_KEY_ID \
  -e AWS_SECRET_ACCESS_KEY=$AWS_SECRET_ACCESS_KEY \
  -e AWS_REGION=us-east-1 \
  ghcr.io/huseyinbabal/taws

# Build locally
docker build -t taws .
docker run --rm -it -v ~/.aws:/root/.aws:ro taws
```

> **Note:** Use `-it` flags for interactive terminal support (required for TUI). Mount your `~/.aws` directory as read-only to use your existing AWS credentials.

### From Source

taws is built with Rust. Make sure you have Rust 1.70+ installed, along with a C compiler and linker.

#### Build Dependencies

| Platform | Install Command |
|----------|-----------------|
| **Amazon Linux / RHEL / Fedora** | `sudo yum groupinstall "Development Tools" -y` |
| **Ubuntu / Debian** | `sudo apt update && sudo apt install build-essential -y` |
| **macOS** | `xcode-select --install` |
| **Windows** | Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) |

```bash
# Clone the repository
git clone https://github.com/huseyinbabal/taws.git
cd taws

# Build and run
cargo build --release
./target/release/taws
```

---

## Prerequisites

- **AWS Credentials** - See [Authentication](#authentication) section below
- **IAM Permissions** - Your AWS user/role needs appropriate read permissions for the services you want to browse. At minimum, you'll need `Describe*` and `List*` permissions.

---

## Authentication

taws uses a credential chain, trying each source in order:

| Priority | Source | Description |
|----------|--------|-------------|
| 1 | Environment Variables | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` |
| 2 | **AWS SSO** | If profile has SSO configured, uses SSO (prompts for login if needed) |
| 3 | **Role Assumption** | If profile has `role_arn` + `source_profile`, assumes the role |
| 4 | Credentials File | `~/.aws/credentials` |
| 5 | Config File | `~/.aws/config` |
| 6 | IMDSv2 | EC2 instance metadata |

### AWS SSO

taws supports AWS SSO. If your profile uses SSO and the token is expired, taws will prompt you to authenticate via browser.

Both SSO config formats are supported:
- Modern: `sso_session` reference to `[sso-session X]` section
- Legacy: `sso_start_url` directly in profile

If you already logged in via `aws sso login`, taws will use the cached token automatically.

### IAM Role Assumption

taws supports assuming IAM roles using `role_arn` with either `source_profile` or `credential_source`. This is commonly used for:
- Cross-account access (e.g., dev account assuming role in prod account)
- Least-privilege access patterns
- Chained role assumption
- Container-based deployments (ECS, Lambda)

#### Using source_profile

Reference another named profile for source credentials:

```ini
[profile base]
region = us-east-1

[profile production]
role_arn = arn:aws:iam::123456789012:role/ProductionAccess
source_profile = base
region = us-west-2

# Optional: external_id for cross-account trust
[profile partner-account]
role_arn = arn:aws:iam::987654321098:role/PartnerAccess
source_profile = base
external_id = my-external-id
```

#### Using credential_source

Load source credentials from environment, EC2 metadata, or ECS container:

```ini
# For ECS tasks with task IAM roles
[profile ecs-admin]
role_arn = arn:aws:iam::123456789012:role/AdminRole
credential_source = EcsContainer

# For EC2 instances with instance roles
[profile ec2-admin]
role_arn = arn:aws:iam::123456789012:role/AdminRole
credential_source = Ec2InstanceMetadata

# For environments with AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY set
[profile env-admin]
role_arn = arn:aws:iam::123456789012:role/AdminRole
credential_source = Environment
```

**Supported credential_source values:**

| Value | Description |
|-------|-------------|
| `Environment` | Load from `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` |
| `Ec2InstanceMetadata` | Load from EC2 instance metadata (IMDSv2) |
| `EcsContainer` | Load from ECS container credentials endpoint |

**Supported options:**

| Option | Required | Description |
|--------|----------|-------------|
| `role_arn` | Yes | ARN of the IAM role to assume |
| `source_profile` | One of | Profile to use for source credentials |
| `credential_source` | these | Where to load source credentials from |
| `external_id` | No | External ID for cross-account trust policies |
| `role_session_name` | No | Custom session name (default: `taws-session`) |
| `duration_seconds` | No | Session duration in seconds (default: 3600) |
| `region` | No | Region for STS endpoint |

**Notes:**
- Use exactly one of `source_profile` OR `credential_source` (not both)
- Chained role assumption is supported (source_profile can also use role_arn)
- Temporary credentials are cached and automatically refreshed before expiration
- ECS container credentials require `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` or `AWS_CONTAINER_CREDENTIALS_FULL_URI` environment variables (set automatically by ECS)

---

## Quick Start

```bash
# Launch taws with default profile
taws

# Launch with a specific profile
taws --profile production

# Launch in a specific region
taws --region us-west-2

# Enable debug logging
taws --log-level debug

# Run in read-only mode (blocks all write operations)
taws --readonly

# Use with LocalStack or custom endpoint
taws --endpoint-url http://localhost:4566

# Or via environment variable
AWS_ENDPOINT_URL=http://localhost:4566 taws
```

### Log File Locations

| Platform | Path |
|----------|------|
| **Linux** | `~/.config/taws/taws.log` |
| **macOS** | `~/Library/Application Support/taws/taws.log` |
| **Windows** | `%APPDATA%\taws\taws.log` |

---

## Key Bindings

| Action | Key | Description |
|--------|-----|-------------|
| **Navigation** | | |
| Move up | `k` / `↑` | Move selection up |
| Move down | `j` / `↓` | Move selection down |
| Top | `gg` / `Home` | Jump to first item |
| Bottom | `G` / `End` | Jump to last item |
| Page up | `PgUp` / `Ctrl+b` | Scroll up one page |
| Page down | `PgDn` / `Ctrl+f` | Scroll down one page |
| **Pagination** | | |
| Next page | `]` | Load next page of results |
| Previous page | `[` | Load previous page of results |
| **Views** | | |
| Resource picker | `:` | Open resource type selector |
| Describe | `Enter` / `d` | View resource details |
| Back | `Esc` / `Backspace` | Go back to previous view |
| Help | `?` | Show help screen |
| **Actions** | | |
| Refresh | `R` | Refresh current view (resets pagination) |
| Filter | `/` | Filter resources |
| Region shortcuts | `0-5` | Quick switch to common regions |
| Quit | `Ctrl-c` | Exit taws |
| **EC2 Actions** | | |
| Connect (SSM) | `c` | Open SSM shell session to instance |
| Start instance | `s` | Start selected EC2 instance |
| Stop instance | `S` | Stop selected EC2 instance |
| Terminate | `Ctrl+d` | Terminate selected EC2 instance |

---

## Resource Navigation

Press `:` to open the resource picker. Type to filter resources:

```
:ec2          # EC2 Instances
:lambda       # Lambda Functions
:s3           # S3 Buckets
:rds          # RDS Instances
:iam-users    # IAM Users
:eks          # EKS Clusters
```

Use `Tab` to autocomplete and `Enter` to select.

---

## Supported AWS Services

taws supports **30 AWS services** with **49 resource types** covering 95%+ of typical AWS usage:

| Category | Service | Resources |
|----------|---------|-----------|
| **Compute** | EC2 | Instances |
| | Lambda | Functions |
| | ECS | Clusters, Services, Tasks |
| | EKS | Clusters |
| | Auto Scaling | Auto Scaling Groups |
| **Storage** | S3 | Buckets |
| **Database** | RDS | Instances, Snapshots |
| | DynamoDB | Tables |
| | ElastiCache | Clusters |
| **Networking** | VPC | VPCs, Subnets, Security Groups |
| | ELBv2 | Load Balancers, Listeners, Rules, Target Groups, Targets |
| | Route 53 | Hosted Zones |
| | CloudFront | Distributions |
| | API Gateway | REST APIs |
| **Security** | IAM | Users, Groups, Roles, Policies, Access Keys |
| | Secrets Manager | Secrets |
| | KMS | Keys |
| | ACM | Certificates |
| | Cognito | User Pools |
| **Management** | CloudFormation | Stacks |
| | CloudWatch | Log Groups |
| | CloudTrail | Trails |
| | SSM | Parameters |
| | STS | Caller Identity |
| **Messaging** | SQS | Queues |
| | SNS | Topics |
| | EventBridge | Event Buses, Rules |
| **Containers** | ECR | Repositories |
| **DevOps** | CodePipeline | Pipelines |
| | CodeBuild | Projects |
| **Analytics** | Athena | Workgroups |

> **Missing a service?** [Start a discussion](https://github.com/huseyinbabal/taws/discussions/new?category=ideas) to propose adding it!

---

## Configuration

See [Authentication](#authentication) for credential setup.

### Environment Variables

| Variable | Description |
|----------|-------------|
| `AWS_PROFILE` | Default AWS profile to use |
| `AWS_REGION` | Default AWS region |
| `AWS_DEFAULT_REGION` | Fallback region (if `AWS_REGION` not set) |
| `AWS_ACCESS_KEY_ID` | AWS access key |
| `AWS_SECRET_ACCESS_KEY` | AWS secret key |
| `AWS_SESSION_TOKEN` | AWS session token (for temporary credentials) |
| `AWS_SHARED_CREDENTIALS_FILE` | Custom path to credentials file (default: `~/.aws/credentials`) |
| `AWS_CONFIG_FILE` | Custom path to config file (default: `~/.aws/config`) |
| `AWS_ENDPOINT_URL` | Custom endpoint URL (for LocalStack, etc.) - also used for STS AssumeRole |
| `AWS_CA_BUNDLE` | Custom CA certificate bundle (PEM format) for corporate SSL inspection |
| `SSL_CERT_FILE` | Alternative to `AWS_CA_BUNDLE` for custom CA certificates |

### Corporate Proxy / SSL Inspection

If you're behind a corporate proxy with SSL inspection, taws may fail to connect to AWS services because the proxy's CA certificate is not trusted by default.

To fix this, set `AWS_CA_BUNDLE` or `SSL_CERT_FILE` to point to your corporate CA certificate bundle:

```bash
# Windows
set AWS_CA_BUNDLE=C:\path\to\corporate-ca-bundle.pem
taws

# Linux/macOS
export AWS_CA_BUNDLE=/path/to/corporate-ca-bundle.pem
taws
```

The PEM file can contain multiple certificates (certificate chain). taws will load all certificates from the bundle and add them to the trusted root certificates.

**Note:** This is the same environment variable used by AWS CLI, so if AWS CLI works with your CA bundle, taws should work too.

---

## SSM Connect (EC2 Shell Access)

Press `c` on a running EC2 instance to open an interactive shell session via AWS Systems Manager.

**Requirements:**
- [session-manager-plugin](https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html) must be installed
- EC2 instance must have SSM Agent running
- Instance must be running (not stopped/terminated)
- Linux instances only (Windows not supported via shell)

**Note:** When you exit the shell session (`exit`), you'll return to taws.

---

## Known Issues

- Some resources may require specific IAM permissions not covered by basic read-only policies
- Total resource count is not displayed due to AWS API limitations (most AWS APIs don't return total count)
- Some global services (IAM, Route53, CloudFront) always use us-east-1

---

## Contributing

Contributions are welcome! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

**Important:** Before adding a new AWS service, please [start a discussion](https://github.com/huseyinbabal/taws/discussions/new?category=ideas) first.

---

## Acknowledgments

- Inspired by [k9s](https://github.com/derailed/k9s) - the awesome Kubernetes CLI
- Built with [Ratatui](https://github.com/ratatui-org/ratatui) - Rust TUI library
- Uses [aws-sigv4](https://github.com/awslabs/aws-sdk-rust) for request signing

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

<p align="center">
  Made with ❤️ for the AWS community
</p>
