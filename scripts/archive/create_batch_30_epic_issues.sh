#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates 12 additional GitHub EPIC issues (200 Points each) for Stellar-K8s.
These are major architectural features requiring significant effort.

Prerequisites:
  - gh CLI installed and authenticated
  - Network access to api.github.com

Optional environment variables:
  REPO                Target repository (default: OtowoOrg/Stellar-K8s)
  DRY_RUN             Set to 1 to print commands without executing

Example:
  REPO=myorg/my-fork DRY_RUN=1 $(basename "$0")
EOF
}

for arg in "$@"; do
  case "$arg" in
    -h|--help) show_help; exit 0 ;;
  esac
done

EXPECTED_ISSUE_COUNT=12
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT." >&2
  exit 1
fi

echo "Creating Batch 30 of 12 EPIC (200-point) issues..."

# EPIC 13: Service Mesh Integration
gh issue create --repo "$REPO" \
  --title "[EPIC] Service Mesh Integration with Advanced Traffic Management" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement deep integration with service mesh platforms (Istio, Linkerd, Consul) to provide advanced traffic management, mutual TLS, circuit breaking, rate limiting, and observability for Stellar services.

## Business Value
- **Enhanced security**: Automatic mTLS between all services
- **Traffic control**: Fine-grained traffic shaping and routing
- **Resilience**: Circuit breakers and retry policies
- **Observability**: Deep insights into service-to-service communication

## Core Requirements
1. **Service Mesh Integration** - Support for Istio, Linkerd, and Consul Connect
2. **Automatic mTLS** - Zero-trust networking with automatic certificate rotation
3. **Traffic Management** - Traffic splitting, mirroring, and routing rules
4. **Resilience Patterns** - Circuit breakers, retries, timeouts, bulkheads
5. **Rate Limiting** - Per-service and per-endpoint rate limiting
6. **Observability** - Service mesh metrics and distributed tracing
7. **Multi-Cluster Mesh** - Federation across multiple Kubernetes clusters

## Acceptance Criteria
- [ ] StellarServiceMesh CRD implemented
- [ ] Support for Istio, Linkerd, and Consul
- [ ] Automatic mTLS enabled and working
- [ ] Traffic splitting for canary deployments
- [ ] Circuit breakers prevent cascade failures
- [ ] Rate limiting enforced per service
- [ ] Service mesh metrics integrated with Prometheus
- [ ] Multi-cluster mesh working across 3+ clusters
- [ ] Grafana dashboard for service mesh topology
- [ ] Documentation with service mesh best practices

## References
- [Istio](https://istio.io/)
- [Linkerd](https://linkerd.io/)
- [Consul Connect](https://www.consul.io/docs/connect)
" --label "epic,200-points,networking,security"

# EPIC 14: Advanced Backup & Restore System
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Backup & Restore with Incremental Snapshots" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build sophisticated backup and restore system with incremental snapshots, deduplication, compression, encryption, multi-cloud support, and automated restore testing to ensure data protection and business continuity.

## Business Value
- **Data protection**: Never lose data with continuous backups
- **Cost efficiency**: 70% storage reduction through deduplication
- **Fast recovery**: Incremental backups enable rapid restore
- **Compliance**: Meet retention and encryption requirements

## Core Requirements
1. **Incremental Snapshots** - Only backup changed data since last snapshot
2. **Deduplication** - Block-level deduplication to reduce storage
3. **Compression** - Intelligent compression algorithms (zstd, lz4)
4. **Encryption** - AES-256 encryption at rest and in transit
5. **Multi-Cloud Support** - S3, GCS, Azure Blob, MinIO
6. **Automated Testing** - Regular restore tests to verify backups
7. **Backup Analytics** - Track backup size, growth, and efficiency

## Acceptance Criteria
- [ ] StellarBackupPolicy CRD implemented
- [ ] Incremental backups working (block-level changes)
- [ ] Deduplication reduces storage by >60%
- [ ] Compression reduces size by >40%
- [ ] Encryption with customer-managed keys
- [ ] Support for S3, GCS, and Azure Blob
- [ ] Automated restore testing weekly
- [ ] Backup verification and integrity checks
- [ ] Grafana dashboard for backup analytics
- [ ] Point-in-time restore working
- [ ] Backup retention policies enforced
- [ ] Documentation with backup strategies

## References
- [Restic](https://restic.net/)
- [Velero](https://velero.io/)
- [Kopia](https://kopia.io/)
" --label "epic,200-points,disaster-recovery,reliability"

# EPIC 15: Intelligent Resource Scheduling
gh issue create --repo "$REPO" \
  --title "[EPIC] Intelligent Resource Scheduling with ML-Based Bin Packing" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement ML-based intelligent scheduler that optimizes pod placement considering network topology, SCP consensus requirements, resource affinity, cost, and power consumption for maximum efficiency.

## Business Value
- **Cost reduction**: 30-40% reduction through optimal placement
- **Performance**: Lower latency through topology-aware scheduling
- **Efficiency**: Better resource utilization (>80%)
- **Sustainability**: Reduce power consumption

## Core Requirements
1. **ML-Based Scheduler** - Learn optimal placement from historical data
2. **Network-Aware Scheduling** - Consider inter-pod latency
3. **Consensus-Aware Placement** - Optimize for SCP message propagation
4. **Cost-Aware Scheduling** - Prefer cheaper nodes/zones
5. **Power-Aware Scheduling** - Minimize power consumption
6. **Affinity Rules** - Support for complex affinity constraints
7. **Real-Time Optimization** - Dynamic pod rescheduling

## Acceptance Criteria
- [ ] Custom Kubernetes scheduler implemented
- [ ] ML model for placement optimization
- [ ] Network latency considered in scheduling
- [ ] SCP consensus times improved by >20%
- [ ] Resource utilization >80%
- [ ] Cost reduction >30% demonstrated
- [ ] Power consumption tracking
- [ ] Dynamic rescheduling working
- [ ] Grafana dashboard for scheduler decisions
- [ ] A/B testing vs default scheduler
- [ ] Documentation with scheduling strategies

## References
- [Kubernetes Scheduler](https://kubernetes.io/docs/concepts/scheduling-eviction/kube-scheduler/)
- [Volcano Scheduler](https://volcano.sh/)
" --label "epic,200-points,performance,kubernetes"

# EPIC 16: Advanced Database Management
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Database Management with Query Optimization" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build comprehensive database management system for Horizon's PostgreSQL including automated query optimization, connection pooling, read replicas, automatic failover, and performance tuning.

## Business Value
- **Performance**: 10x faster queries through optimization
- **Scalability**: Handle 100x more concurrent connections
- **Reliability**: Zero-downtime during database failures
- **Cost**: 50% cost reduction through efficient resource usage

## Core Requirements
1. **Query Optimization** - Automatic slow query detection and optimization
2. **Connection Pooling** - Advanced connection pooling with PgBouncer
3. **Read Replicas** - Automatic read replica management and routing
4. **Automatic Failover** - Zero-downtime failover with Patroni
5. **Performance Tuning** - Auto-tuning of PostgreSQL parameters
6. **Database Monitoring** - Deep insights into database performance
7. **Schema Migration** - Zero-downtime schema migrations

## Acceptance Criteria
- [ ] StellarDatabase CRD implemented
- [ ] Slow query detection and alerts
- [ ] Automatic index recommendations
- [ ] PgBouncer connection pooling
- [ ] Read replica auto-scaling (1-10 replicas)
- [ ] Automatic failover with Patroni
- [ ] PostgreSQL auto-tuning working
- [ ] Query performance improved by >5x
- [ ] Connection pool efficiency >90%
- [ ] Zero-downtime schema migrations
- [ ] Grafana dashboard for database metrics
- [ ] Documentation with database optimization guide

## References
- [PostgreSQL Performance](https://wiki.postgresql.org/wiki/Performance_Optimization)
- [PgBouncer](https://www.pgbouncer.org/)
- [Patroni](https://patroni.readthedocs.io/)
" --label "epic,200-points,performance,reliability"

# EPIC 17: Container Registry Management
gh issue create --repo "$REPO" \
  --title "[EPIC] Container Registry Management with Security Scanning" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive container registry management with automated security scanning, image signing, vulnerability remediation, registry mirroring, and garbage collection.

## Business Value
- **Security**: Prevent vulnerable images from running
- **Compliance**: Meet image scanning requirements
- **Efficiency**: Reduce storage costs by 60%
- **Reliability**: Registry mirroring ensures availability

## Core Requirements
1. **Security Scanning** - Automated CVE scanning with Trivy/Grype
2. **Image Signing** - Cosign/Notary for image verification
3. **Vulnerability Remediation** - Automatic patching of base images
4. **Registry Mirroring** - Multi-region registry replication
5. **Garbage Collection** - Automated cleanup of unused images
6. **Registry Proxy** - Caching proxy for external registries
7. **Admission Control** - Block unsigned or vulnerable images

## Acceptance Criteria
- [ ] StellarRegistry CRD implemented
- [ ] Trivy/Grype scanning integrated
- [ ] All images signed with Cosign
- [ ] Admission webhook blocks vulnerable images
- [ ] Automatic base image patching
- [ ] Registry mirroring across 3+ regions
- [ ] Garbage collection reduces storage by >50%
- [ ] Registry proxy for Docker Hub
- [ ] Image vulnerability dashboard
- [ ] Compliance reports (CVE count, severity)
- [ ] Documentation with registry best practices

## References
- [Harbor](https://goharbor.io/)
- [Trivy](https://github.com/aquasecurity/trivy)
- [Cosign](https://github.com/sigstore/cosign)
" --label "epic,200-points,security,compliance"

# EPIC 18: Advanced Secret Management
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Secret Management with Dynamic Secrets" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build sophisticated secret management system with dynamic secret generation, automatic rotation, secret encryption, access audit logging, and integration with enterprise secret stores.

## Business Value
- **Security**: Eliminate long-lived credentials
- **Compliance**: Audit trail for all secret access
- **Automation**: No manual secret rotation
- **Integration**: Work with existing secret stores

## Core Requirements
1. **Dynamic Secrets** - Generate secrets on-demand with TTL
2. **Automatic Rotation** - Rotate secrets without downtime
3. **Secret Encryption** - Encrypt secrets at rest and in transit
4. **Access Auditing** - Complete audit trail of secret access
5. **Multiple Backends** - Vault, AWS Secrets Manager, Azure Key Vault
6. **Secret Injection** - Inject secrets as files or environment variables
7. **Secret Versioning** - Track secret versions and rollback

## Acceptance Criteria
- [ ] StellarSecret CRD implemented
- [ ] Dynamic database credentials working
- [ ] Automatic rotation every 30 days
- [ ] Secrets encrypted with KMS
- [ ] Complete audit logging
- [ ] Integration with Vault, AWS, Azure
- [ ] Zero-downtime rotation
- [ ] Secret versioning and rollback
- [ ] Grafana dashboard for secret metrics
- [ ] Compliance reports for auditors
- [ ] Documentation with secret management guide

## References
- [HashiCorp Vault](https://www.vaultproject.io/)
- [External Secrets Operator](https://external-secrets.io/)
- [Sealed Secrets](https://github.com/bitnami-labs/sealed-secrets)
" --label "epic,200-points,security,compliance"

# EPIC 19: Kubernetes Operator SDK Framework
gh issue create --repo "$REPO" \
  --title "[EPIC] Kubernetes Operator SDK Framework for Extensions" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Create extensible SDK framework that allows developers to build custom operators and extensions for Stellar-K8s using a well-defined API, reducing code duplication and accelerating development.

## Business Value
- **Extensibility**: Enable community contributions
- **Productivity**: 10x faster operator development
- **Consistency**: Standardized patterns and best practices
- **Maintenance**: Reduce code duplication by 60%

## Core Requirements
1. **SDK Library** - Reusable components for operator development
2. **Code Generation** - Generate boilerplate from CRD definitions
3. **Testing Framework** - Built-in testing utilities and mocks
4. **Plugin System** - Support for custom controllers and webhooks
5. **CLI Tools** - CLI for scaffolding and development
6. **Documentation Generator** - Auto-generate API docs
7. **Example Operators** - Reference implementations

## Acceptance Criteria
- [ ] SDK library published to crates.io
- [ ] Code generator working (CRD → controller)
- [ ] Testing framework with integration test support
- [ ] Plugin system for custom controllers
- [ ] CLI tool for scaffolding operators
- [ ] Auto-generated API documentation
- [ ] 3+ example operators
- [ ] Comprehensive developer guide
- [ ] Tutorial for building first operator
- [ ] CI/CD templates for operators

## References
- [Operator SDK](https://sdk.operatorframework.io/)
- [Kubebuilder](https://book.kubebuilder.io/)
- [kube-rs](https://kube.rs/)
" --label "epic,200-points,dx,architecture"

# EPIC 20: Advanced Networking Policies
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Networking Policies with Microsegmentation" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement advanced networking capabilities including microsegmentation, zero-trust networking, network policy automation, egress filtering, and integration with enterprise network security tools.

## Business Value
- **Security**: Zero-trust networking prevents lateral movement
- **Compliance**: Meet network security requirements
- **Automation**: Auto-generate network policies
- **Visibility**: Complete network traffic visibility

## Core Requirements
1. **Microsegmentation** - Pod-to-pod network policies
2. **Zero-Trust Networking** - Deny-all by default with explicit allow
3. **Policy Automation** - Auto-generate policies from traffic patterns
4. **Egress Filtering** - Control and audit egress traffic
5. **DNS Policies** - DNS-based access control
6. **Network Observability** - Traffic flow visualization
7. **Integration** - Calico, Cilium, Antrea support

## Acceptance Criteria
- [ ] StellarNetworkPolicy CRD implemented
- [ ] Microsegmentation enabled
- [ ] Zero-trust policies enforced
- [ ] Automatic policy generation from traffic
- [ ] Egress filtering with audit logs
- [ ] DNS-based access control
- [ ] Traffic flow visualization dashboard
- [ ] Integration with Calico/Cilium
- [ ] Network policy testing framework
- [ ] Compliance reports
- [ ] Documentation with networking guide

## References
- [Calico](https://www.tigera.io/project-calico/)
- [Cilium](https://cilium.io/)
- [Network Policies](https://kubernetes.io/docs/concepts/services-networking/network-policies/)
" --label "epic,200-points,networking,security"

# EPIC 21: Cloud-Native Storage Management
gh issue create --repo "$REPO" \
  --title "[EPIC] Cloud-Native Storage Management with CSI Integration" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build comprehensive storage management system with CSI driver integration, volume lifecycle management, snapshot management, volume cloning, and performance optimization for Stellar workloads.

## Business Value
- **Flexibility**: Support for 10+ storage backends
- **Performance**: 5x faster I/O through optimization
- **Cost**: 40% storage cost reduction
- **Reliability**: Automated volume recovery

## Core Requirements
1. **CSI Driver Integration** - Support for AWS EBS, GCP PD, Azure Disk
2. **Volume Lifecycle** - Automated provisioning, expansion, deletion
3. **Snapshot Management** - Scheduled snapshots with retention
4. **Volume Cloning** - Fast volume cloning for testing
5. **Performance Optimization** - IOPS and throughput tuning
6. **Storage Analytics** - Usage tracking and forecasting
7. **Multi-Tier Storage** - Hot/warm/cold storage tiers

## Acceptance Criteria
- [ ] StellarStorageClass CRD implemented
- [ ] Support for AWS, GCP, Azure storage
- [ ] Automatic volume expansion
- [ ] Scheduled snapshots working
- [ ] Volume cloning in <1 minute
- [ ] Performance tuning (IOPS, throughput)
- [ ] Storage usage analytics
- [ ] Multi-tier storage policies
- [ ] Storage cost optimization
- [ ] Grafana dashboard for storage metrics
- [ ] Documentation with storage best practices

## References
- [Kubernetes CSI](https://kubernetes-csi.github.io/)
- [Rook](https://rook.io/)
- [OpenEBS](https://openebs.io/)
" --label "epic,200-points,reliability,performance"

# EPIC 22: Advanced Logging Pipeline
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Logging Pipeline with Structured Logging" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement sophisticated logging infrastructure with structured logging, log aggregation, real-time analysis, log retention management, and integration with enterprise SIEM systems.

## Business Value
- **Troubleshooting**: 70% faster incident resolution
- **Compliance**: Meet log retention requirements
- **Cost**: 50% reduction in logging costs
- **Security**: Real-time security event detection

## Core Requirements
1. **Structured Logging** - JSON-formatted logs with consistent schema
2. **Log Aggregation** - Centralized logging with Loki/Elasticsearch
3. **Real-Time Analysis** - Stream processing with Flink/Kafka
4. **Log Enrichment** - Add context (pod, node, namespace)
5. **Retention Management** - Tiered storage (hot/warm/cold)
6. **SIEM Integration** - Forward to Splunk/Datadog/Elastic
7. **Log Sampling** - Intelligent sampling to reduce volume

## Acceptance Criteria
- [ ] StellarLogging CRD implemented
- [ ] All logs in structured JSON format
- [ ] Log aggregation with Loki
- [ ] Real-time log analysis
- [ ] Automatic log enrichment
- [ ] Tiered retention (7d hot, 30d warm, 90d cold)
- [ ] SIEM integration working
- [ ] Log sampling reduces volume by >60%
- [ ] Full-text search with <1s latency
- [ ] Grafana dashboard for log analytics
- [ ] Documentation with logging best practices

## References
- [Grafana Loki](https://grafana.com/oss/loki/)
- [Fluentd](https://www.fluentd.org/)
- [Vector](https://vector.dev/)
" --label "epic,200-points,observability,monitoring"

# EPIC 23: Chaos Engineering Platform
gh issue create --repo "$REPO" \
  --title "[EPIC] Chaos Engineering Platform with Automated Game Days" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build comprehensive chaos engineering platform that automates resilience testing through game days, chaos experiments, blast radius control, and automated recovery validation.

## Business Value
- **Resilience**: Prove system reliability before production
- **Confidence**: 10x more confident in deployments
- **Learning**: Identify weaknesses proactively
- **Compliance**: Meet chaos testing requirements

## Core Requirements
1. **Chaos Experiments** - Network, pod, node, disk, CPU, memory chaos
2. **Automated Game Days** - Scheduled chaos testing
3. **Blast Radius Control** - Limit impact of chaos experiments
4. **Observability** - Track system behavior during chaos
5. **Automated Recovery** - Validate recovery procedures
6. **Chaos Scenarios** - Library of common failure scenarios
7. **Reporting** - Generate resilience reports

## Acceptance Criteria
- [ ] StellarChaos CRD implemented
- [ ] 10+ chaos experiment types
- [ ] Automated game days (weekly)
- [ ] Blast radius control working
- [ ] Real-time observability during chaos
- [ ] Automated recovery validation
- [ ] 20+ pre-built scenarios
- [ ] Resilience scoring system
- [ ] Grafana dashboard for chaos results
- [ ] PDF reports for compliance
- [ ] Documentation with chaos engineering guide

## References
- [Chaos Mesh](https://chaos-mesh.org/)
- [LitmusChaos](https://litmuschaos.io/)
- [Principles of Chaos Engineering](https://principlesofchaos.org/)
" --label "epic,200-points,reliability,testing"

# EPIC 24: Developer Experience Platform
gh issue create --repo "$REPO" \
  --title "[EPIC] Developer Experience Platform with Local Development" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Create comprehensive developer experience platform with local development environment, hot-reload capabilities, debugging tools, and seamless integration with production infrastructure.

## Business Value
- **Productivity**: 5x faster development cycles
- **Onboarding**: New developers productive in <1 day
- **Quality**: Catch issues before production
- **Satisfaction**: Happy developers write better code

## Core Requirements
1. **Local Development** - Run full Stellar stack locally
2. **Hot Reload** - See changes instantly without rebuild
3. **Remote Debugging** - Debug production pods from IDE
4. **Port Forwarding** - Access cluster services locally
5. **Log Streaming** - Stream logs to local terminal
6. **Resource Templates** - Quick-start templates for common tasks
7. **IDE Integration** - VS Code and IntelliJ plugins

## Acceptance Criteria
- [ ] Local development environment (Tilt/Skaffold)
- [ ] Hot reload working (<5s iteration time)
- [ ] Remote debugging with VS Code
- [ ] Port forwarding for all services
- [ ] Real-time log streaming
- [ ] 10+ resource templates
- [ ] VS Code extension published
- [ ] Developer onboarding <1 hour
- [ ] Developer satisfaction >90%
- [ ] Comprehensive developer guide
- [ ] Video tutorials

## References
- [Tilt](https://tilt.dev/)
- [Skaffold](https://skaffold.dev/)
- [Telepresence](https://www.telepresence.io/)
" --label "epic,200-points,dx,automation"

echo "✅ Created 12 additional EPIC (200-point) issues successfully!"
