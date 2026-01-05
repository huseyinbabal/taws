<p align="center">
  <img src="assets/taws-logo.png" alt="taws" width="400"/>
</p>

# taws - Terminal UI for AWS

**taws** provides a terminal UI to interact with your AWS resources. The aim of this project is to make it easier to navigate, observe, and manage your AWS infrastructure in the wild. taws continually watches AWS for changes and offers subsequent commands to interact with your observed resources.

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
- **Real-time Updates** - Refresh resources with a single keystroke
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
| **Linux** | x86_64 | `taws-x86_64-unknown-linux-gnu.tar.gz` |
| **Linux** | ARM64 | `taws-aarch64-unknown-linux-gnu.tar.gz` |
| **Windows** | x86_64 | `taws-x86_64-pc-windows-msvc.zip` |

#### Quick Install (macOS/Linux)

```bash
# macOS Apple Silicon
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-aarch64-apple-darwin.tar.gz | tar xz
sudo mv taws /usr/local/bin/

# macOS Intel
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-x86_64-apple-darwin.tar.gz | tar xz
sudo mv taws /usr/local/bin/

# Linux x86_64
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv taws /usr/local/bin/

# Linux ARM64
curl -sL https://github.com/huseyinbabal/taws/releases/latest/download/taws-aarch64-unknown-linux-gnu.tar.gz | tar xz
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

- **AWS Credentials** - Configure your AWS credentials using one of:
  - `aws configure` (AWS CLI)
  - Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
  - IAM roles (when running on EC2/ECS/Lambda)
  - AWS profiles in `~/.aws/credentials`

- **IAM Permissions** - Your AWS user/role needs appropriate read permissions for the services you want to browse. At minimum, you'll need `Describe*` and `List*` permissions.

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
| Page up | `Ctrl-u` | Move up by page |
| Page down | `Ctrl-d` | Move down by page |
| Top | `g` | Jump to first item |
| Bottom | `G` | Jump to last item |
| **Views** | | |
| Resource picker | `:` | Open resource type selector |
| Describe | `Enter` / `d` | View resource details |
| Back | `Esc` | Go back to previous view |
| Help | `?` | Show help screen |
| **Actions** | | |
| Refresh | `r` | Refresh current view |
| Filter | `/` | Filter resources |
| Profiles | `p` | Switch AWS profile |
| Regions | `R` | Switch AWS region |
| Quit | `q` / `Ctrl-c` | Exit taws |
| **EC2 Actions** | | |
| Start instance | `s` | Start selected EC2 instance |
| Stop instance | `S` | Stop selected EC2 instance |
| Terminate | `T` | Terminate selected EC2 instance |

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

taws supports **30 core AWS services** covering 95%+ of typical AWS usage:

| Category | Services |
|----------|----------|
| **Compute** | EC2, Lambda, ECS, EKS, Auto Scaling |
| **Storage** | S3 |
| **Database** | RDS, DynamoDB, ElastiCache |
| **Networking** | VPC (Subnets, Security Groups), Route 53, CloudFront, API Gateway, ELB |
| **Security** | IAM, Secrets Manager, KMS, ACM, Cognito |
| **Management** | CloudFormation, CloudWatch Logs, CloudTrail, SSM, STS |
| **Messaging** | SQS, SNS, EventBridge |
| **Containers** | ECR |
| **DevOps** | CodePipeline, CodeBuild |
| **Analytics** | Athena |

> **Missing a service?** [Start a discussion](https://github.com/huseyinbabal/taws/discussions/new?category=ideas) to propose adding it!

---

## Configuration

taws looks for AWS credentials in the standard locations:
- `~/.aws/credentials`
- `~/.aws/config`
- Environment variables

### Environment Variables

| Variable | Description |
|----------|-------------|
| `AWS_PROFILE` | Default AWS profile to use |
| `AWS_REGION` | Default AWS region |
| `AWS_ACCESS_KEY_ID` | AWS access key |
| `AWS_SECRET_ACCESS_KEY` | AWS secret key |
| `AWS_SESSION_TOKEN` | AWS session token (for temporary credentials) |

---

## Known Issues

- Some resources may require specific IAM permissions not covered by basic read-only policies
- Resource counts may vary during loading due to pagination
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
