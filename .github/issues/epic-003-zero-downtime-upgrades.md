# [EPIC] Zero-Downtime Stellar Core Upgrades with Canary Deployments

**Labels:** `epic`, `200-points`, `reliability`, `upgrades`

## Epic Overview

Implement a sophisticated upgrade system that enables zero-downtime upgrades of Stellar Core, Horizon, and Soroban RPC nodes using canary deployments, automated rollback, and progressive traffic shifting. This ensures production systems can upgrade safely without service interruption or consensus participation gaps.

## Business Value

- **Zero service interruption**: Maintain 100% uptime during upgrades
- **Risk mitigation**: Detect issues before full rollout
- **Faster releases**: Confidence to upgrade more frequently
- **Compliance**: Meet SLA requirements for critical infrastructure

## Scope & Requirements

### Core Requirements

1. **Canary Deployment Strategy**
   - Deploy new version to small subset of nodes (10-20%)
   - Monitor canary health for configurable duration
   - Automatically promote or rollback based on metrics
   - Support for multi-stage rollouts (10% → 50% → 100%)

2. **Automated Health Validation**
   - Consensus participation verification (validators)
   - Ledger sync status monitoring
   - API response time and error rate tracking
   - Database migration validation
   - Peer connectivity checks

3. **Progressive Traffic Shifting**
   - Gradual traffic migration to upgraded nodes
   - Weighted routing based on version
   - Automatic traffic drain for rollback
   - Support for A/B testing different versions

4. **Intelligent Rollback**
   - Automatic rollback on health check failures
   - Manual rollback via kubectl or API
   - Preserve data during rollback
   - Rollback time < 2 minutes

5. **Upgrade Coordination**
   - Coordinate upgrades across node types (Core → Horizon → Soroban)
   - Respect dependency order
   - Handle database schema migrations
   - Quorum-aware validator upgrades (never break consensus)

6. **Upgrade Policies**
   - Maintenance windows for scheduled upgrades
   - Emergency upgrade path for security patches
   - Approval gates for production environments
   - Notification system for upgrade events

## Technical Design

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│              Upgrade Orchestrator                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │   Canary     │  │   Health     │  │   Traffic    │  │
│  │   Manager    │  │   Validator  │  │   Shifter    │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────────────────────────────────────────────────┘
         │                    │                    │
         ▼                    ▼                    ▼
┌─────────────────────────────────────────────────────────┐
│                  Node Versions                           │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │  v21.0.0     │  │  v21.1.0     │  │  v21.1.0     │  │
│  │  (Stable)    │  │  (Canary)    │  │  (Promoted)  │  │
│  │  90% traffic │  │  10% traffic │  │  100% traffic│  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### New CRD: `StellarUpgrade`

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarUpgrade
metadata:
  name: upgrade-to-v21-1-0
spec:
  targetRef:
    apiVersion: stellar.org/v1alpha1
    kind: StellarNode
    name: my-horizon
  
  fromVersion: "v21.0.0"
  toVersion: "v21.1.0"
  
  strategy:
    type: Canary  # Canary | BlueGreen | RollingUpdate
    
    canary:
      steps:
        - setWeight: 10
          pause:
            duration: 10m
        - setWeight: 50
          pause:
            duration: 20m
        - setWeight: 100
      
      analysis:
        interval: 1m
        threshold: 3  # failures before rollback
        metrics:
          - name: error-rate
            successCondition: result < 0.05
            provider:
              prometheus:
                query: |
                  rate(stellar_horizon_errors_total{version="v21.1.0"}[5m])
          
          - name: ledger-lag
            successCondition: result < 10
            provider:
              prometheus:
                query: |
                  stellar_horizon_ledger_lag{version="v21.1.0"}
          
          - name: response-time-p95
            successCondition: result < 500
            provider:
              prometheus:
                query: |
                  histogram_quantile(0.95, 
                    rate(stellar_horizon_request_duration_ms_bucket{version="v21.1.0"}[5m]))
      
      trafficRouting:
        istio:
          virtualService:
            name: horizon-vsvc
            routes:
              - primary
              - canary
  
  approvalPolicy:
    required: true
    approvers:
      - team: platform-engineering
        minApprovals: 2
    timeout: 24h
  
  maintenanceWindow:
    start: "2026-06-15T02:00:00Z"
    end: "2026-06-15T06:00:00Z"
    timezone: "UTC"
  
  rollbackPolicy:
    automatic: true
    onFailure: true
    preserveData: true
  
  notifications:
    slack:
      channel: "#stellar-ops"
      events:
        - UpgradeStarted
        - CanaryPromoted
        - UpgradeCompleted
        - RollbackTriggered
    email:
      recipients:
        - ops@example.com
      events:
        - UpgradeFailed
        - RollbackCompleted
```

### Implementation Components

1. **Upgrade Controller**
   - Watch `StellarUpgrade` resources
   - Orchestrate multi-stage rollouts
   - Coordinate with traffic management
   - Handle approval workflows

2. **Canary Manager**
   - Create canary deployments
   - Manage version coexistence
   - Track canary health metrics
   - Execute promotion/rollback decisions

3. **Health Validator**
   - Query Prometheus for metrics
   - Execute custom health checks
   - Aggregate health scores
   - Trigger rollback on failures

4. **Traffic Shifter**
   - Integrate with Istio/Linkerd for traffic splitting
   - Update Service weights
   - Manage DNS records for external traffic
   - Implement connection draining

5. **Database Migration Handler**
   - Detect schema changes
   - Run migrations before upgrade
   - Verify migration success
   - Rollback migrations if needed

6. **Quorum Coordinator (Validators)**
   - Ensure quorum maintained during upgrades
   - Upgrade validators one at a time
   - Wait for consensus participation before next
   - Emergency stop if quorum at risk

7. **Notification System**
   - Slack/Teams/Email integrations
   - Webhook support for custom integrations
   - Event streaming to audit log

## Acceptance Criteria

- [ ] `StellarUpgrade` CRD implemented with full validation
- [ ] Canary deployments working for all node types
- [ ] Automated health validation with 5+ metrics
- [ ] Progressive traffic shifting (10% → 50% → 100%)
- [ ] Automatic rollback on health check failures
- [ ] Manual rollback via kubectl command
- [ ] Approval workflow for production upgrades
- [ ] Maintenance window enforcement
- [ ] Validator upgrades maintain quorum (tested with 5-node cluster)
- [ ] Database migration handling
- [ ] Slack/email notifications
- [ ] Grafana dashboard showing upgrade progress
- [ ] Documentation with upgrade runbooks
- [ ] E2E tests for successful upgrade path
- [ ] E2E tests for rollback scenarios
- [ ] Performance benchmarks (upgrade time, rollback time)
- [ ] Helm chart support for upgrade resources

## Dependencies & Blockers

- Requires service mesh (Istio/Linkerd) for traffic splitting
- Needs Prometheus for health metrics
- May require database migration tool (Flyway/Liquibase)
- Approval workflow needs integration with identity provider

## Testing Strategy

### Unit Tests
- Upgrade state machine logic
- Health check evaluation
- Traffic weight calculations
- Rollback decision logic

### Integration Tests
- Canary deployment creation
- Traffic routing configuration
- Database migration execution
- Notification delivery

### E2E Tests
- Full upgrade cycle (v21.0.0 → v21.1.0)
- Rollback on health check failure
- Rollback on manual trigger
- Upgrade with database migration
- Validator upgrade maintaining quorum
- Upgrade during high traffic

### Chaos Tests
- Network partition during upgrade
- Node failure during canary phase
- Metrics endpoint unavailable
- Database migration failure
- Quorum loss during validator upgrade

### Performance Tests
- Upgrade time for 10-node cluster
- Rollback time measurement
- Traffic shifting latency
- Resource overhead during upgrade

## Estimated Effort

**200 Story Points** (~6-8 weeks for 2 engineers)

## Related Issues

- #TBD: Istio/Linkerd integration
- #TBD: Database migration framework
- #TBD: Approval workflow system
- #TBD: Upgrade notification system

## References

- [Argo Rollouts](https://argoproj.github.io/argo-rollouts/)
- [Flagger (Flux CD)](https://flagger.app/)
- [Istio Traffic Management](https://istio.io/latest/docs/concepts/traffic-management/)
- [Kubernetes Blue-Green Deployments](https://kubernetes.io/blog/2018/04/30/zero-downtime-deployment-kubernetes-jenkins/)
- [Stellar Protocol Upgrades](https://developers.stellar.org/docs/learn/fundamentals/stellar-consensus-protocol#protocol-upgrades)
