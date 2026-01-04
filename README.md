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

### From Source

taws is built with Rust. Make sure you have Rust 1.70+ installed.

```bash
# Clone the repository
git clone https://github.com/huseyinbabal/taws.git
cd taws

# Build and run
cargo build --release
./target/release/taws
```

### Using Cargo

```bash
cargo install taws
```

### Homebrew (macOS/Linux)

```bash
brew install huseyinbabal/tap/taws
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
```

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

## AWS Service Coverage

taws currently supports **68 AWS services** with **100+ resource types**:

✅ = Implemented | ⏳ = Planned

### Compute

| Service | Status | Resources |
|---------|:------:|-----------|
| EC2 | ✅ | Instances, Security Groups, Volumes, Snapshots, AMIs, Key Pairs |
| Lambda | ✅ | Functions |
| ECS | ✅ | Clusters, Services, Tasks |
| EKS | ✅ | Clusters |
| Elastic Beanstalk | ✅ | Applications, Environments |
| App Runner | ✅ | Services |
| Batch | ✅ | Job Queues, Compute Environments |
| Lightsail | ✅ | Instances |
| Auto Scaling | ✅ | Groups |
| EC2 Image Builder | ⏳ | |
| Outposts | ⏳ | |

### Containers & Serverless

| Service | Status | Resources |
|---------|:------:|-----------|
| ECR | ✅ | Repositories |
| Fargate | ✅ | (via ECS) |
| Step Functions | ✅ | State Machines |
| EventBridge | ✅ | Rules, Event Buses |

### Storage

| Service | Status | Resources |
|---------|:------:|-----------|
| S3 | ✅ | Buckets |
| EFS | ✅ | File Systems |
| FSx | ✅ | File Systems |
| Storage Gateway | ✅ | Gateways |
| Backup | ✅ | Backup Vaults, Plans |
| S3 Glacier | ⏳ | |
| Snow Family | ⏳ | |

### Database

| Service | Status | Resources |
|---------|:------:|-----------|
| RDS | ✅ | Instances, Snapshots |
| DynamoDB | ✅ | Tables |
| ElastiCache | ✅ | Clusters |
| Neptune | ✅ | Clusters |
| MemoryDB | ✅ | Clusters |
| Redshift | ✅ | Clusters |
| DocumentDB | ⏳ | |
| Keyspaces | ⏳ | |
| Timestream | ⏳ | |
| QLDB | ⏳ | |

### Networking & Content Delivery

| Service | Status | Resources |
|---------|:------:|-----------|
| VPC | ✅ | VPCs, Subnets, Route Tables, NAT Gateways, Internet Gateways |
| CloudFront | ✅ | Distributions |
| Route 53 | ✅ | Hosted Zones |
| API Gateway | ✅ | REST APIs |
| Direct Connect | ✅ | Connections |
| Global Accelerator | ⏳ | |
| PrivateLink | ⏳ | |
| Transit Gateway | ⏳ | |
| App Mesh | ⏳ | |

### Security, Identity & Compliance

| Service | Status | Resources |
|---------|:------:|-----------|
| IAM | ✅ | Users, Roles, Policies, Groups, Access Keys |
| Secrets Manager | ✅ | Secrets |
| KMS | ✅ | Keys |
| ACM | ✅ | Certificates |
| WAF | ✅ | Web ACLs, IP Sets |
| Shield | ✅ | Protections |
| GuardDuty | ✅ | Detectors |
| Inspector | ✅ | Findings |
| Cognito | ✅ | User Pools |
| Security Hub | ⏳ | |
| Macie | ⏳ | |
| Detective | ⏳ | |
| Firewall Manager | ⏳ | |
| Resource Access Manager | ⏳ | |

### Management & Governance

| Service | Status | Resources |
|---------|:------:|-----------|
| CloudFormation | ✅ | Stacks |
| CloudTrail | ✅ | Trails |
| CloudWatch | ✅ | Log Groups, Alarms |
| Config | ✅ | Rules, Recorders |
| Organizations | ✅ | Accounts |
| SSM | ✅ | Parameters, Documents |
| STS | ✅ | Caller Identity |
| Budgets | ✅ | Budgets |
| Cost Explorer | ⏳ | |
| Service Catalog | ⏳ | |
| Trusted Advisor | ⏳ | |
| Control Tower | ⏳ | |
| License Manager | ⏳ | |
| Health | ⏳ | |

### Developer Tools

| Service | Status | Resources |
|---------|:------:|-----------|
| CodeBuild | ✅ | Projects |
| CodePipeline | ✅ | Pipelines |
| X-Ray | ✅ | Groups, Sampling Rules |
| CodeCommit | ⏳ | |
| CodeDeploy | ⏳ | |
| CodeArtifact | ⏳ | |
| Cloud9 | ⏳ | |
| CodeStar | ⏳ | |

### Analytics

| Service | Status | Resources |
|---------|:------:|-----------|
| Athena | ✅ | Work Groups, Data Catalogs |
| EMR | ✅ | Clusters |
| Kinesis | ✅ | Streams |
| Firehose | ✅ | Delivery Streams |
| Glue | ✅ | Databases, Jobs, Crawlers |
| QuickSight | ✅ | Dashboards |
| OpenSearch | ✅ | Domains |
| MSK | ⏳ | |
| Data Pipeline | ⏳ | |
| Lake Formation | ⏳ | |
| CloudSearch | ⏳ | |

### Machine Learning

| Service | Status | Resources |
|---------|:------:|-----------|
| SageMaker | ✅ | Endpoints, Notebook Instances |
| Bedrock | ✅ | Models, Custom Models |
| Rekognition | ⏳ | |
| Comprehend | ⏳ | |
| Polly | ⏳ | |
| Transcribe | ⏳ | |
| Translate | ⏳ | |
| Lex | ⏳ | |
| Personalize | ⏳ | |
| Forecast | ⏳ | |
| Textract | ⏳ | |
| Kendra | ⏳ | |

### Application Integration

| Service | Status | Resources |
|---------|:------:|-----------|
| SNS | ✅ | Topics |
| SQS | ✅ | Queues |
| AppSync | ✅ | APIs |
| MQ | ✅ | Brokers |
| Amazon MQ | ✅ | (ActiveMQ, RabbitMQ) |
| SWF | ⏳ | |

### Media Services

| Service | Status | Resources |
|---------|:------:|-----------|
| MediaConvert | ✅ | Job Templates, Queues |
| MediaLive | ⏳ | |
| MediaPackage | ⏳ | |
| Elemental | ⏳ | |
| IVS | ⏳ | |

### Migration & Transfer

| Service | Status | Resources |
|---------|:------:|-----------|
| DMS | ✅ | Replication Instances, Tasks |
| DataSync | ✅ | Tasks, Locations |
| Transfer Family | ✅ | Servers |
| Migration Hub | ⏳ | |
| Application Discovery | ⏳ | |
| Server Migration | ⏳ | |

### End User Computing

| Service | Status | Resources |
|---------|:------:|-----------|
| WorkSpaces | ✅ | Workspaces, Directories |
| AppStream 2.0 | ⏳ | |
| WorkDocs | ⏳ | |
| WorkLink | ⏳ | |

### Front-End Web & Mobile

| Service | Status | Resources |
|---------|:------:|-----------|
| Amplify | ✅ | Apps |
| SES | ✅ | Identities |
| Pinpoint | ⏳ | |
| Device Farm | ⏳ | |

### IoT

| Service | Status | Resources |
|---------|:------:|-----------|
| IoT Core | ⏳ | |
| IoT Greengrass | ⏳ | |
| IoT Analytics | ⏳ | |
| IoT Events | ⏳ | |
| IoT SiteWise | ⏳ | |

### Game Development

| Service | Status | Resources |
|---------|:------:|-----------|
| GameLift | ⏳ | |
| Lumberyard | ⏳ | |

---

## Architecture

taws follows a data-driven architecture where AWS resource definitions are stored as JSON configuration files. This makes it easy to add new resource types without writing new code.

```
src/
├── resources/          # JSON resource definitions (one per service)
│   ├── ec2.json
│   ├── lambda.json
│   ├── s3.json
│   └── ...
├── resource/
│   ├── registry.rs     # Resource registry and loading
│   ├── fetcher.rs      # Generic resource fetcher
│   └── sdk_dispatch.rs # AWS SDK dispatch (the only SDK code)
├── aws/
│   ├── client.rs       # AWS client management
│   └── profiles.rs     # AWS profile handling
└── ui/
    ├── table.rs        # Resource table view
    ├── details.rs      # Resource details view
    └── ...
```

### Adding a New Resource Type

1. Add a JSON definition in `src/resources/<service>.json`
2. Add the SDK client to `src/aws/client.rs`
3. Add the SDK dispatch handler to `src/resource/sdk_dispatch.rs`

No other code changes required!

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

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Development

```bash
# Run in development mode
cargo run

# Run tests
cargo test

# Check formatting
cargo fmt --check

# Run linter
cargo clippy
```

---

## Acknowledgments

- Inspired by [k9s](https://github.com/derailed/k9s) - the awesome Kubernetes CLI
- Built with [Ratatui](https://github.com/ratatui-org/ratatui) - Rust TUI library
- Uses [AWS SDK for Rust](https://github.com/awslabs/aws-sdk-rust)

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

<p align="center">
  Made with ❤️ for the AWS community
</p>
