#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates 12 GitHub EPIC issues (200 Points each) for Stellar-K8s.
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

echo "Creating Batch of 12 EPIC (200-point) issues..."

# EPIC 1: Multi-Region Federation
gh issue create --repo "$REPO" \
  --title "[EPIC] Multi-Region Federation Support with Automated Failover" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive multi-region federation support that enables Stellar nodes to operate across multiple Kubernetes clusters in different geographic regions with automated failover, cross-region replication, and intelligent traffic routing.

## Business Value
- **Zero-downtime deployments**: Automatic failover during regional outages
- **Reduced latency**: Route users to nearest healthy region
- **Compliance**: Meet data residency requirements
- **Disaster recovery**: Automated recovery from catastrophic failures

## Core Requirements
1. **Multi-Cluster CRD Synchronization** - Sync StellarNode resources across federated clusters
2. **Cross-Region Service Discovery** - Automatic peer discovery across regions
3. **Intelligent Traffic Routing** - Geographic load balancing with health-aware routing
4. **Automated Failover** - Detect regional failures within 30 seconds
5. **Data Replication Strategy** - History archive and PostgreSQL replication
6. **Federation Control Plane** - Central management API for federated clusters

## Acceptance Criteria
- [ ] StellarFederation CRD implemented with full validation
- [ ] Federation controller can manage 3+ regions simultaneously
- [ ] Automatic failover completes within 60 seconds
- [ ] Zero transaction loss during planned failover
- [ ] Cross-region health monitoring with Prometheus metrics
- [ ] Grafana dashboard showing federation topology
- [ ] E2E tests simulating regional failures
- [ ] RTO < 60 seconds, RPO < 5 seconds

## References
- [Kubernetes Federation v2](https://github.com/kubernetes-sigs/kubefed)
- [Istio Multi-Cluster](https://istio.io/latest/docs/setup/install/multicluster/)
" --label "epic,200-points,high-availability,phase-3"

# EPIC 2: Advanced Autoscaling
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Autoscaling with Predictive Scaling and Custom Metrics" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement intelligent autoscaling for Horizon and Soroban RPC nodes using machine learning for predictive scaling, custom Stellar-specific metrics (TPS, ledger lag, contract invocations), and cost-aware scaling policies.

## Business Value
- **Cost optimization**: 30-50% cost reduction through intelligent scaling
- **Performance guarantee**: Proactive scaling before traffic spikes
- **Better user experience**: Maintain consistent API response times
- **Resource efficiency**: Right-size deployments based on actual workload

## Core Requirements
1. **Custom Metrics Autoscaling** - Scale based on TPS, ledger lag, RPC queue depth
2. **Predictive Scaling** - ML-based traffic prediction 5-15 minutes in advance
3. **Cost-Aware Scaling** - Prefer cost-effective scaling strategies
4. **Multi-Dimensional Scaling** - Independent scaling for different node types
5. **Scaling Policies** - Aggressive, conservative, balanced, custom
6. **Integration with HPA/KEDA** - Extend Kubernetes HPA with custom metrics

## Acceptance Criteria
- [ ] StellarAutoscaler CRD implemented
- [ ] Support for 5+ custom Stellar metrics
- [ ] Predictive scaling with >70% accuracy
- [ ] Cost tracking and budget enforcement
- [ ] Schedule-based scaling working
- [ ] Integration with Kubernetes HPA and KEDA
- [ ] Grafana dashboard showing autoscaling decisions
- [ ] E2E tests simulating traffic patterns

## References
- [Kubernetes HPA](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
- [KEDA](https://keda.sh/)
- [Prophet Forecasting](https://facebook.github.io/prophet/)
" --label "epic,200-points,performance,autoscaling"

# EPIC 3: Zero-Downtime Upgrades
gh issue create --repo "$REPO" \
  --title "[EPIC] Zero-Downtime Stellar Core Upgrades with Canary Deployments" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement sophisticated upgrade system enabling zero-downtime upgrades of Stellar Core, Horizon, and Soroban RPC using canary deployments, automated rollback, and progressive traffic shifting.

## Business Value
- **Zero service interruption**: Maintain 100% uptime during upgrades
- **Risk mitigation**: Detect issues before full rollout
- **Faster releases**: Confidence to upgrade more frequently
- **Compliance**: Meet SLA requirements for critical infrastructure

## Core Requirements
1. **Canary Deployment Strategy** - Deploy to subset, monitor, promote/rollback
2. **Automated Health Validation** - Consensus, sync status, API metrics
3. **Progressive Traffic Shifting** - Gradual migration (10% → 50% → 100%)
4. **Intelligent Rollback** - Automatic rollback on health failures
5. **Upgrade Coordination** - Coordinate Core → Horizon → Soroban upgrades
6. **Upgrade Policies** - Maintenance windows, approval gates, notifications

## Acceptance Criteria
- [ ] StellarUpgrade CRD implemented
- [ ] Canary deployments for all node types
- [ ] Automated health validation with 5+ metrics
- [ ] Progressive traffic shifting working
- [ ] Automatic rollback on failures
- [ ] Validator upgrades maintain quorum
- [ ] Database migration handling
- [ ] Slack/email notifications
- [ ] Rollback time < 2 minutes

## References
- [Argo Rollouts](https://argoproj.github.io/argo-rollouts/)
- [Flagger](https://flagger.app/)
- [Istio Traffic Management](https://istio.io/latest/docs/concepts/traffic-management/)
" --label "epic,200-points,reliability,upgrades"

# EPIC 4: Observability Platform
gh issue create --repo "$REPO" \
  --title "[EPIC] Comprehensive Observability Platform with Distributed Tracing" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build complete observability platform with distributed tracing across all components, advanced log aggregation, real-time alerting with intelligent noise reduction, and AI-powered anomaly detection.

## Business Value
- **Faster incident resolution**: Reduce MTTR by 60-80%
- **Proactive issue detection**: Identify problems before users affected
- **Performance optimization**: Pinpoint bottlenecks across the stack
- **Cost visibility**: Track resource usage and optimize spending

## Core Requirements
1. **Distributed Tracing** - End-to-end request tracing with OpenTelemetry
2. **Advanced Log Aggregation** - Centralized logs with structured querying
3. **Intelligent Alerting** - Multi-condition rules with ML-based fatigue reduction
4. **Anomaly Detection** - ML-based baseline learning and pattern recognition
5. **Performance Profiling** - Continuous profiling with flame graphs
6. **Custom Dashboards** - Pre-built dashboards for all node types
7. **Cost Attribution** - Per-node cost tracking and forecasting

## Acceptance Criteria
- [ ] StellarObservability CRD implemented
- [ ] Distributed tracing end-to-end (API → Core)
- [ ] Log aggregation with full-text search
- [ ] 10+ pre-built alert rules
- [ ] Anomaly detection with >80% accuracy
- [ ] Continuous profiling for CPU and memory
- [ ] Cost attribution per node
- [ ] 5+ pre-built Grafana dashboards
- [ ] Tracing overhead < 5%

## References
- [OpenTelemetry](https://opentelemetry.io/)
- [Grafana Tempo](https://grafana.com/oss/tempo/)
- [Grafana Loki](https://grafana.com/oss/loki/)
" --label "epic,200-points,observability,monitoring"

# EPIC 5: Disaster Recovery Automation
gh issue create --repo "$REPO" \
  --title "[EPIC] Automated Disaster Recovery with Point-in-Time Restore" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive disaster recovery automation enabling point-in-time restore from history archives, automated backup verification, DR drills, and cross-region failover with RPO < 5 minutes.

## Business Value
- **Business continuity**: Recover from catastrophic failures in minutes
- **Data protection**: Prevent data loss with continuous backups
- **Compliance**: Meet regulatory requirements for DR testing
- **Risk mitigation**: Regular DR drills ensure procedures work

## Core Requirements
1. **Automated Backup Management** - Continuous backup to history archives
2. **Point-in-Time Restore** - Restore to any ledger number or timestamp
3. **Disaster Recovery Drills** - Scheduled automated DR tests
4. **Backup Verification** - Automated integrity checks and restore testing
5. **Cross-Region Failover** - Automatic failover to DR region
6. **Recovery Time Optimization** - Parallel restore, pre-warmed standby
7. **Backup Lifecycle Management** - Retention policies and cleanup

## Acceptance Criteria
- [ ] StellarBackup, StellarRestore, StellarDRDrill CRDs implemented
- [ ] Automated backups to S3/GCS
- [ ] Point-in-time restore to specific ledger
- [ ] Backup encryption and cross-region replication
- [ ] Automated backup verification
- [ ] DR drill execution and reporting
- [ ] RTO < 30 minutes, RPO < 5 minutes
- [ ] PDF drill reports with compliance metrics

## References
- [Stellar History Archives](https://developers.stellar.org/docs/run-core-node/publishing-history-archives)
- [Velero](https://velero.io/)
" --label "epic,200-points,disaster-recovery,phase-3"

# EPIC 6: Security & Compliance Framework
gh issue create --repo "$REPO" \
  --title "[EPIC] Security & Compliance Framework with Automated Auditing" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive security and compliance framework with automated security scanning, policy enforcement, compliance auditing (SOC2, PCI-DSS, GDPR), secret management, and continuous security monitoring.

## Business Value
- **Risk reduction**: Prevent security breaches and data leaks
- **Compliance**: Meet SOC2, PCI-DSS, GDPR requirements
- **Audit readiness**: Automated compliance reports
- **Trust**: Demonstrate security posture to customers

## Core Requirements
1. **Automated Security Scanning** - Container, dependency, configuration scanning
2. **Policy Enforcement** - Pod Security Standards, network policies, RBAC
3. **Secret Management** - Vault/AWS Secrets Manager integration with rotation
4. **Compliance Auditing** - SOC2, PCI-DSS, GDPR compliance reporting
5. **Network Security** - mTLS, network segmentation, DDoS protection
6. **Access Control** - RBAC, OIDC/SAML, MFA, audit logging
7. **Security Monitoring** - Real-time threat detection, SIEM integration

## Acceptance Criteria
- [ ] StellarSecurityPolicy CRD implemented
- [ ] Automated vulnerability scanning
- [ ] Pod Security Standards enforcement
- [ ] Network policies auto-generated
- [ ] Secret management with Vault
- [ ] Automated secret rotation
- [ ] SOC2 and GDPR compliance reporting
- [ ] mTLS between all components
- [ ] Security monitoring with real-time alerts
- [ ] Penetration testing report

## References
- [Pod Security Standards](https://kubernetes.io/docs/concepts/security/pod-security-standards/)
- [HashiCorp Vault](https://www.vaultproject.io/)
- [Trivy Scanner](https://github.com/aquasecurity/trivy)
" --label "epic,200-points,security,compliance"

# EPIC 7: GitOps Integration
gh issue create --repo "$REPO" \
  --title "[EPIC] GitOps Integration with Progressive Delivery" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive GitOps integration with ArgoCD and Flux CD, enabling declarative infrastructure management, automated synchronization, progressive delivery, drift detection, and self-healing.

## Business Value
- **Infrastructure as Code**: Version-controlled, auditable infrastructure
- **Faster deployments**: Automated, consistent deployments
- **Reduced errors**: Eliminate manual configuration mistakes
- **Audit trail**: Complete history of all changes

## Core Requirements
1. **ArgoCD Integration** - Auto-discovery, sync policies, health assessment
2. **Flux CD Integration** - GitRepository source, Kustomization, HelmRelease
3. **Progressive Delivery** - Canary deployments with Flagger
4. **Drift Detection** - Detect manual changes and alert
5. **Self-Healing** - Automatic sync on drift
6. **Multi-Environment Management** - Dev, staging, production workflows
7. **Secret Management** - Sealed Secrets, SOPS integration

## Acceptance Criteria
- [ ] StellarGitOpsConfig CRD implemented
- [ ] ArgoCD and Flux CD integration working
- [ ] Automated sync from Git
- [ ] Drift detection and alerting
- [ ] Self-healing on drift
- [ ] Canary deployments with Flagger
- [ ] Multi-environment management
- [ ] Promotion workflows with approvals
- [ ] Sealed Secrets integration

## References
- [ArgoCD](https://argo-cd.readthedocs.io/)
- [Flux CD](https://fluxcd.io/)
- [Flagger](https://flagger.app/)
" --label "epic,200-points,gitops,ci-cd"

# EPIC 8: AIOps Platform
gh issue create --repo "$REPO" \
  --title "[EPIC] AIOps Platform with Intelligent Incident Management" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Build AI-powered operations platform using machine learning for intelligent incident detection, root cause analysis, automated remediation, capacity planning, and predictive maintenance.

## Business Value
- **Reduced MTTR**: AI-powered RCA cuts resolution time by 70%
- **Proactive prevention**: Predict and prevent issues before impact
- **Operational efficiency**: Automate 60-80% of routine tasks
- **24/7 operations**: Automated incident response without on-call

## Core Requirements
1. **Intelligent Incident Detection** - Multi-signal anomaly detection
2. **Root Cause Analysis** - Automated RCA using causal inference
3. **Automated Remediation** - Runbook automation with self-healing
4. **Capacity Planning** - Resource usage forecasting and recommendations
5. **Predictive Maintenance** - Predict component failures
6. **Intelligent Alerting** - Alert prioritization and fatigue reduction
7. **ChatOps Integration** - Slack/Teams bot for incident management

## Acceptance Criteria
- [ ] StellarAIOps CRD implemented
- [ ] Anomaly detection with >85% accuracy
- [ ] Root cause analysis with >70% confidence
- [ ] Automated remediation for 5+ common issues
- [ ] Capacity forecasting 90 days ahead
- [ ] Predictive maintenance for disk exhaustion
- [ ] ChatOps bot with 10+ commands
- [ ] Alert prioritization and deduplication
- [ ] Knowledge base with incident history

## References
- [AIOps Overview](https://www.gartner.com/en/information-technology/glossary/aiops-artificial-intelligence-operations)
- [Prophet Forecasting](https://facebook.github.io/prophet/)
" --label "epic,200-points,ai-ops,automation"

# EPIC 9: Performance Optimization Framework
gh issue create --repo "$REPO" \
  --title "[EPIC] Performance Optimization Framework with Continuous Benchmarking" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive performance optimization framework with continuous benchmarking, performance regression detection, automated profiling, query optimization, caching strategies, and resource tuning.

## Business Value
- **Cost reduction**: 30-50% reduction through optimization
- **Better user experience**: Faster API response times
- **Scalability**: Handle 10x traffic with same infrastructure
- **Resource efficiency**: Maximize utilization

## Core Requirements
1. **Continuous Benchmarking** - Automated performance tests on every deployment
2. **Automated Profiling** - Continuous CPU, memory, I/O profiling
3. **Query Optimization** - Slow query detection and index recommendations
4. **Caching Strategy** - Multi-tier caching (L1, L2, CDN)
5. **Resource Tuning** - Automatic CPU/memory/network tuning
6. **Load Testing** - Realistic traffic simulation and stress testing
7. **Performance Budgets** - Define SLOs and track against budgets

## Acceptance Criteria
- [ ] StellarPerformance CRD implemented
- [ ] Automated benchmarking on every deployment
- [ ] Performance regression detection
- [ ] Continuous CPU and memory profiling
- [ ] Slow query detection and recommendations
- [ ] Multi-tier caching with >80% hit rate
- [ ] Automated resource tuning
- [ ] 30% improvement in API latency (p95)
- [ ] 50% improvement in throughput

## References
- [Pyroscope](https://grafana.com/oss/pyroscope/)
- [k6 Load Testing](https://k6.io/)
" --label "epic,200-points,performance,optimization"

# EPIC 10: Advanced Network Topology Management
gh issue create --repo "$REPO" \
  --title "[EPIC] Advanced Network Topology Management with SCP Analytics" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement advanced network topology management that provides real-time SCP message analysis, quorum health monitoring, peer relationship optimization, and network partition detection for Stellar validator networks.

## Business Value
- **Network health**: Real-time visibility into consensus health
- **Faster consensus**: Optimize peer relationships for lower latency
- **Partition detection**: Early detection of network splits
- **Compliance**: Meet validator network requirements

## Core Requirements
1. **SCP Message Streaming** - High-throughput streaming to Kafka
2. **Topology Visualization** - Real-time network graph visualization
3. **Quorum Health Monitoring** - Track quorum set health and changes
4. **Peer Optimization** - Recommend optimal peer configurations
5. **Partition Detection** - Detect and alert on network partitions
6. **Historical Analysis** - Query historical SCP data
7. **Network Simulation** - Simulate topology changes

## Acceptance Criteria
- [ ] SCP message streaming to Kafka working
- [ ] Real-time topology visualization dashboard
- [ ] Quorum health metrics and alerts
- [ ] Peer optimization recommendations
- [ ] Network partition detection within 30 seconds
- [ ] Historical SCP data queryable
- [ ] Network simulation tool
- [ ] Documentation with topology best practices

## References
- [Stellar Consensus Protocol](https://developers.stellar.org/docs/learn/fundamentals/stellar-consensus-protocol)
- [Apache Kafka](https://kafka.apache.org/)
" --label "epic,200-points,networking,scp-analytics"

# EPIC 11: Multi-Tenancy Platform
gh issue create --repo "$REPO" \
  --title "[EPIC] Multi-Tenancy Platform with Resource Isolation" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement comprehensive multi-tenancy platform enabling multiple teams/organizations to share Stellar infrastructure with strong resource isolation, quota management, cost allocation, and tenant-specific policies.

## Business Value
- **Cost efficiency**: Share infrastructure across teams
- **Resource isolation**: Prevent noisy neighbor problems
- **Simplified management**: Centralized platform for multiple tenants
- **Chargeback**: Accurate cost allocation per tenant

## Core Requirements
1. **Tenant Isolation** - Namespace-based isolation with network policies
2. **Resource Quotas** - Per-tenant CPU, memory, storage quotas
3. **Cost Allocation** - Track and report costs per tenant
4. **Tenant Policies** - Custom security and compliance policies per tenant
5. **Self-Service Portal** - Web UI for tenant management
6. **Hierarchical Tenancy** - Support for sub-tenants and organizations
7. **Tenant Monitoring** - Per-tenant metrics and dashboards

## Acceptance Criteria
- [ ] StellarTenant CRD implemented
- [ ] Namespace-based tenant isolation
- [ ] Resource quotas enforced per tenant
- [ ] Cost allocation and reporting
- [ ] Tenant-specific security policies
- [ ] Self-service portal for tenant management
- [ ] Hierarchical tenancy support
- [ ] Per-tenant Grafana dashboards
- [ ] Documentation with multi-tenancy guide

## References
- [Kubernetes Multi-Tenancy](https://kubernetes.io/docs/concepts/security/multi-tenancy/)
- [Hierarchical Namespaces](https://github.com/kubernetes-sigs/hierarchical-namespaces)
" --label "epic,200-points,multi-tenancy,platform"

# EPIC 12: Intelligent Capacity Management
gh issue create --repo "$REPO" \
  --title "[EPIC] Intelligent Capacity Management with Cost Optimization" \
  --body "### 🔴 Difficulty: Hard (200 Points)

## Epic Overview
Implement intelligent capacity management system that uses ML for resource forecasting, provides cost optimization recommendations, enables what-if scenario planning, and automates capacity provisioning.

## Business Value
- **Cost optimization**: 40-60% cost reduction through right-sizing
- **Prevent outages**: Proactive capacity provisioning
- **Budget planning**: Accurate capacity and cost forecasts
- **Resource efficiency**: Eliminate over-provisioning

## Core Requirements
1. **Resource Forecasting** - ML-based forecasting for CPU, memory, disk, network
2. **Cost Optimization** - Recommendations for instance types, spot instances
3. **What-If Scenarios** - Model impact of traffic changes
4. **Automated Provisioning** - Auto-provision capacity based on forecasts
5. **Capacity Alerts** - Alert when approaching capacity limits
6. **Historical Analysis** - Track capacity trends over time
7. **Budget Management** - Set and track capacity budgets

## Acceptance Criteria
- [ ] StellarCapacity CRD implemented
- [ ] Resource forecasting 90 days ahead with >80% accuracy
- [ ] Cost optimization recommendations
- [ ] What-if scenario modeling
- [ ] Automated capacity provisioning
- [ ] Capacity alerts working
- [ ] Historical capacity trend analysis
- [ ] Budget tracking and alerts
- [ ] 40% cost reduction demonstrated

## References
- [AWS Cost Explorer](https://aws.amazon.com/aws-cost-management/aws-cost-explorer/)
- [Prophet Forecasting](https://facebook.github.io/prophet/)
" --label "epic,200-points,capacity-management,cost-optimization"

echo "✅ Created 12 EPIC (200-point) issues successfully!"
