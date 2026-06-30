# [EPIC] GitOps Integration with Progressive Delivery

**Labels:** `epic`, `200-points`, `gitops`, `ci-cd`

## Epic Overview

Implement comprehensive GitOps integration with ArgoCD and Flux CD, enabling declarative infrastructure management, automated synchronization, progressive delivery with canary and blue-green deployments, drift detection, and self-healing capabilities. This brings modern DevOps practices to Stellar infrastructure management.

## Business Value

- **Infrastructure as Code**: Version-controlled, auditable infrastructure
- **Faster deployments**: Automated, consistent deployments
- **Reduced errors**: Eliminate manual configuration mistakes
- **Audit trail**: Complete history of all changes
- **Disaster recovery**: Recreate infrastructure from Git
- **Collaboration**: Team-based infrastructure management

## Scope & Requirements

### Core Requirements

1. **ArgoCD Integration**
   - Auto-discovery of `StellarNode` resources
   - Sync policies (automatic, manual, selective)
   - Health assessment for Stellar resources
   - Custom resource actions (sync, restart, scale)
   - Multi-cluster management
   - ApplicationSet for templating

2. **Flux CD Integration**
   - GitRepository source integration
   - Kustomization for Stellar resources
   - HelmRelease for operator deployment
   - Image automation for version updates
   - Notification system integration
   - Multi-tenancy support

3. **Progressive Delivery**
   - Canary deployments with Flagger
   - Blue-green deployments
   - A/B testing support
   - Automated rollback on failures
   - Traffic splitting and shifting
   - Metrics-based promotion

4. **Drift Detection**
   - Detect manual changes to resources
   - Alert on configuration drift
   - Auto-remediation options
   - Drift reports and dashboards
   - Compliance enforcement

5. **Self-Healing**
   - Automatic sync on drift
   - Restart failed pods
   - Recreate deleted resources
   - Health-based remediation
   - Configurable healing policies

6. **Multi-Environment Management**
   - Dev, staging, production environments
   - Environment-specific configurations
   - Promotion workflows (dev → staging → prod)
   - Environment parity validation
   - Cost-optimized dev/staging

7. **Secret Management**
   - Sealed Secrets integration
   - SOPS encryption
   - External Secrets Operator
   - Secret rotation automation
   - Audit logging for secrets

## Technical Design

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Git Repository                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │     Dev      │  │   Staging    │  │  Production  │      │
│  │  manifests/  │  │  manifests/  │  │  manifests/  │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
└─────────────────────────────────────────────────────────────┘
         │                  │                  │
         ▼                  ▼                  ▼
┌─────────────────────────────────────────────────────────────┐
│              GitOps Controller (ArgoCD/Flux)                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │    Sync      │  │    Drift     │  │  Progressive │      │
│  │   Engine     │  │   Detector   │  │   Delivery   │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
└─────────────────────────────────────────────────────────────┘
         │                  │                  │
         ▼                  ▼                  ▼
┌─────────────────────────────────────────────────────────────┐
│                  Kubernetes Clusters                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │  Dev Cluster │  │ Staging Clstr│  │  Prod Cluster│      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
└─────────────────────────────────────────────────────────────┘
```

### Repository Structure

```
stellar-infrastructure/
├── base/
│   ├── operator/
│   │   ├── kustomization.yaml
│   │   └── operator.yaml
│   └── crds/
│       └── stellarnode-crd.yaml
├── overlays/
│   ├── dev/
│   │   ├── kustomization.yaml
│   │   ├── horizon.yaml
│   │   └── patches/
│   ├── staging/
│   │   ├── kustomization.yaml
│   │   ├── horizon.yaml
│   │   ├── validator.yaml
│   │   └── patches/
│   └── production/
│       ├── kustomization.yaml
│       ├── horizon.yaml
│       ├── validator.yaml
│       ├── soroban-rpc.yaml
│       └── patches/
├── apps/
│   ├── dev-app.yaml
│   ├── staging-app.yaml
│   └── production-app.yaml
└── progressive-delivery/
    ├── canary-horizon.yaml
    └── blue-green-validator.yaml
```

### ArgoCD Application

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: stellar-production
  namespace: argocd
spec:
  project: stellar
  
  source:
    repoURL: https://github.com/example/stellar-infrastructure
    targetRevision: main
    path: overlays/production
    
    kustomize:
      commonLabels:
        environment: production
        managed-by: argocd
  
  destination:
    server: https://kubernetes.default.svc
    namespace: stellar
  
  syncPolicy:
    automated:
      prune: true
      selfHeal: true
      allowEmpty: false
    
    syncOptions:
      - CreateNamespace=true
      - PrunePropagationPolicy=foreground
      - PruneLast=true
    
    retry:
      limit: 5
      backoff:
        duration: 5s
        factor: 2
        maxDuration: 3m
  
  ignoreDifferences:
    - group: stellar.org
      kind: StellarNode
      jsonPointers:
        - /status
  
  info:
    - name: 'Runbook'
      value: 'https://runbooks.example.com/stellar'
    - name: 'Slack'
      value: '#stellar-ops'
```

### Flux Kustomization

```yaml
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: stellar-production
  namespace: flux-system
spec:
  interval: 5m
  
  sourceRef:
    kind: GitRepository
    name: stellar-infrastructure
  
  path: ./overlays/production
  
  prune: true
  wait: true
  timeout: 10m
  
  healthChecks:
    - apiVersion: stellar.org/v1alpha1
      kind: StellarNode
      name: horizon-production
      namespace: stellar
  
  postBuild:
    substitute:
      CLUSTER_NAME: "prod-us-east-1"
      ENVIRONMENT: "production"
  
  patches:
    - patch: |
        - op: replace
          path: /spec/replicas
          value: 5
      target:
        kind: StellarNode
        name: horizon-production
```

### Flagger Canary

```yaml
apiVersion: flagger.app/v1beta1
kind: Canary
metadata:
  name: horizon-canary
  namespace: stellar
spec:
  targetRef:
    apiVersion: stellar.org/v1alpha1
    kind: StellarNode
    name: horizon-production
  
  progressDeadlineSeconds: 600
  
  service:
    port: 8000
    targetPort: 8000
  
  analysis:
    interval: 1m
    threshold: 5
    maxWeight: 50
    stepWeight: 10
    
    metrics:
      - name: request-success-rate
        thresholdRange:
          min: 99
        interval: 1m
      
      - name: request-duration
        thresholdRange:
          max: 500
        interval: 1m
      
      - name: ledger-lag
        thresholdRange:
          max: 10
        interval: 1m
    
    webhooks:
      - name: load-test
        url: http://flagger-loadtester/
        timeout: 5s
        metadata:
          cmd: "hey -z 1m -q 10 -c 2 http://horizon-canary:8000/health"
```

### New CRD: `StellarGitOpsConfig`

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarGitOpsConfig
metadata:
  name: gitops-config
spec:
  provider: argocd  # argocd | flux
  
  repository:
    url: https://github.com/example/stellar-infrastructure
    branch: main
    path: overlays/production
    
    credentials:
      secretRef:
        name: git-credentials
  
  syncPolicy:
    automated: true
    prune: true
    selfHeal: true
    syncInterval: 5m
  
  driftDetection:
    enabled: true
    checkInterval: 1m
    alertOnDrift: true
    autoRemediate: true
    
    ignoredFields:
      - /status
      - /metadata/managedFields
  
  progressiveDelivery:
    enabled: true
    provider: flagger  # flagger | argo-rollouts
    
    defaultStrategy:
      type: canary
      steps:
        - setWeight: 10
        - pause: {duration: 5m}
        - setWeight: 50
        - pause: {duration: 10m}
        - setWeight: 100
  
  environments:
    - name: dev
      cluster: dev-cluster
      namespace: stellar-dev
      autoSync: true
    
    - name: staging
      cluster: staging-cluster
      namespace: stellar-staging
      autoSync: true
      requiresApproval: false
    
    - name: production
      cluster: prod-cluster
      namespace: stellar
      autoSync: false
      requiresApproval: true
      approvers:
        - team: platform-engineering
  
  notifications:
    slack:
      channel: "#stellar-gitops"
      events:
        - SyncSucceeded
        - SyncFailed
        - DriftDetected
        - HealthDegraded
```

### Implementation Components

1. **GitOps Controller**
   - Watch `StellarGitOpsConfig` resources
   - Create ArgoCD/Flux resources
   - Manage sync policies
   - Handle multi-environment deployments

2. **Drift Detector**
   - Compare Git state with cluster state
   - Detect manual changes
   - Generate drift reports
   - Trigger auto-remediation

3. **Progressive Delivery Manager**
   - Create Flagger/Argo Rollouts resources
   - Monitor canary health
   - Execute traffic shifting
   - Handle rollbacks

4. **Environment Promoter**
   - Automate dev → staging → prod promotions
   - Validate environment parity
   - Handle approval workflows
   - Track promotion history

5. **Secret Manager**
   - Integrate with Sealed Secrets/SOPS
   - Encrypt secrets in Git
   - Decrypt at runtime
   - Rotate secrets automatically

6. **Metrics and Dashboards**
   ```
   stellar_gitops_sync_status{environment, application}
   stellar_gitops_drift_detected{environment, resource}
   stellar_gitops_sync_duration_seconds{environment}
   stellar_gitops_canary_weight{environment, application}
   stellar_gitops_promotion_total{from_env, to_env}
   ```

## Acceptance Criteria

- [ ] `StellarGitOpsConfig` CRD implemented
- [ ] ArgoCD integration working
- [ ] Flux CD integration working
- [ ] Automated sync from Git
- [ ] Drift detection and alerting
- [ ] Self-healing on drift
- [ ] Canary deployments with Flagger
- [ ] Blue-green deployments
- [ ] Multi-environment management (dev, staging, prod)
- [ ] Promotion workflows with approvals
- [ ] Sealed Secrets integration
- [ ] Grafana GitOps dashboard
- [ ] Documentation with GitOps best practices
- [ ] E2E tests for sync and drift detection
- [ ] E2E tests for canary deployments
- [ ] Performance benchmarks (sync time)
- [ ] Helm chart for GitOps components

## Dependencies & Blockers

- Requires ArgoCD or Flux CD installed
- Needs Flagger for progressive delivery
- Git repository for infrastructure code
- May require Sealed Secrets or SOPS
- Approval workflow needs integration with identity provider

## Testing Strategy

### Unit Tests
- Sync policy evaluation
- Drift detection logic
- Canary promotion decisions
- Environment promotion logic

### Integration Tests
- ArgoCD Application creation
- Flux Kustomization creation
- Flagger Canary creation
- Secret encryption/decryption

### E2E Tests
- Full GitOps sync cycle
- Drift detection and remediation
- Canary deployment with rollback
- Environment promotion (dev → staging → prod)
- Secret rotation

### Chaos Tests
- Git repository unavailable
- ArgoCD/Flux controller down
- Network partition during sync
- Conflicting manual changes
- Canary health check failures

## Estimated Effort

**200 Story Points** (~6-8 weeks for 2 engineers)

## Related Issues

- #TBD: ArgoCD deployment and configuration
- #TBD: Flux CD deployment and configuration
- #TBD: Flagger integration
- #TBD: Sealed Secrets setup

## References

- [ArgoCD](https://argo-cd.readthedocs.io/)
- [Flux CD](https://fluxcd.io/)
- [Flagger](https://flagger.app/)
- [Sealed Secrets](https://github.com/bitnami-labs/sealed-secrets)
- [SOPS](https://github.com/mozilla/sops)
- [GitOps Principles](https://opengitops.dev/)
