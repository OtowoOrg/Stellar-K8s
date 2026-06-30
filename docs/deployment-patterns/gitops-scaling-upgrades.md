# GitOps, Capacity Planning, and Upgrade Strategies

> This guide provides copy-pasteable Terraform and Helm configuration patterns for production rollout governance.

---

## 1) Terraform Baseline (EKS + Node Pools)

```hcl
terraform {
  required_version = ">= 1.6.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.0"
    }
  }
}

provider "aws" {
  region = var.region
}

module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "20.24.0"

  cluster_name    = "stellar-k8s-prod"
  cluster_version = "1.31"

  vpc_id     = var.vpc_id
  subnet_ids = var.private_subnet_ids

  eks_managed_node_groups = {
    validators = {
      instance_types = ["c7i.2xlarge"]
      desired_size   = 3
      min_size       = 3
      max_size       = 5
      labels = {
        "node-role.stellar.io/validator" = "true"
      }
      taints = {
        validator = {
          key    = "stellar.io/validator"
          value  = "true"
          effect = "NO_SCHEDULE"
        }
      }
    }
    rpc = {
      instance_types = ["m7i.2xlarge"]
      desired_size   = 6
      min_size       = 3
      max_size       = 20
      labels = {
        "node-role.stellar.io/rpc" = "true"
      }
    }
  }
}
```

---

## 2) Helm Values Pattern for Stellar-K8s

```yaml
# values-prod.yaml
stellarOperator:
  replicaCount: 2
  leaderElection:
    enabled: true
  resources:
    requests:
      cpu: 500m
      memory: 512Mi
    limits:
      cpu: 1000m
      memory: 1Gi

validator:
  enabled: true
  replicas: 3
  podDisruptionBudget:
    minAvailable: 2
  affinity:
    nodeAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        nodeSelectorTerms:
          - matchExpressions:
              - key: node-role.stellar.io/validator
                operator: In
                values: ["true"]

horizon:
  enabled: true
  replicas: 6
  autoscaling:
    enabled: true
    minReplicas: 6
    maxReplicas: 30
    targetCPUUtilizationPercentage: 65
```

---

## 3) Capacity Planning Matrix

| Component | Baseline CPU | Baseline Memory | Scale Trigger |
|---|---|---|---|
| Validator | 2-4 vCPU | 8-16 GiB | ledger age growth, peer backlog |
| Horizon | 1-2 vCPU | 2-8 GiB | p95 latency, queue depth |
| Operator | 0.5-1 vCPU | 0.5-1 GiB | reconciliation p95 > target |

## 4) Blue-Green and Canary Upgrade Workflow

1. Deploy canary validator set with non-signing mode (observer) for binary validation.
2. Deploy canary RPC subset and route 5% traffic.
3. Gate progression on metrics:

```promql
histogram_quantile(
  0.95,
  sum(rate(stellar_k8s_operator_reconciliation_duration_seconds_bucket[5m])) by (le)
) < 2
```

```promql
max(stellar_core_ledger_age{role="validator"}) < 5
```

4. Promote to blue-green cutover only when burn-rate alerts remain clear for two full evaluation windows.

## 5) GitOps Policy Hooks

- Require signed commits and protected branch rules.
- Block sync if policy checks fail (`conftest`, Gatekeeper dry-run, schema validation).
- Auto-rollback when canary error budget is exceeded.
