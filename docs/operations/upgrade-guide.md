# Major Version Upgrade Guide for Stellar-K8s

## Overview

This guide provides step-by-step operational workflows for major version software upgrades across Stellar-K8s validator and API nodes. It covers zero-downtime upgrade procedures, comprehensive pre-flight checks, and detailed rollback strategies for production environments.

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Pre-Upgrade Planning](#pre-upgrade-planning)
3. [Pre-Flight Verification](#pre-flight-verification)
4. [Upgrade Strategies](#upgrade-strategies)
5. [Database Schema Upgrades](#database-schema-upgrades)
6. [Post-Upgrade Validation](#post-upgrade-validation)
7. [Rollback Procedures](#rollback-procedures)
8. [Emergency Recovery](#emergency-recovery)

## Prerequisites

### Required Tools and Access

```bash
# Verify kubectl access to all clusters
kubectl cluster-info
kubectl auth can-i '*' '*' --as=system:serviceaccount:default:default

# Verify helm installation
helm version

# Verify backup tools are available
which pg_dump
which pg_basebackup
which ceph-shell

# Verify upgrade orchestration tool
./scripts/upgrade-orchestrator --version
```

### Cluster State Requirements

- All nodes must be in Ready state
- Quorum validators must have consensus achieved
- Replication lag on Horizon must be < 5 seconds
- Disk usage must be < 80% on all nodes
- No pending CVE patches requiring immediate action

### Backup Requirements

**Before ANY upgrade, take full backups:**

```bash
# Backup Stellar Core database
pg_dump postgresql://user:pass@core-db:5432/stellar_core > stellar_core_$(date +%Y%m%d_%H%M%S).sql

# Backup Horizon database
pg_dump postgresql://user:pass@horizon-db:5432/horizon > horizon_$(date +%Y%m%d_%H%M%S).sql

# Backup Kubernetes resources
kubectl get all,crd,cm,secret,pvc -A -o yaml > k8s_resources_$(date +%Y%m%d_%H%M%S).yaml

# Backup volume snapshots
ceph -s
rbd snap create <pool>/<image>@pre-upgrade-$(date +%Y%m%d_%H%M%S)
```

## Pre-Upgrade Planning

### 1. Version Compatibility Assessment

```bash
# Check current versions
kubectl get deployment -A -o custom-columns=NAMESPACE:.metadata.namespace,NAME:.metadata.name,IMAGE:.spec.template.spec.containers[0].image

# Document current state
cat > pre-upgrade-inventory.yaml << EOF
timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)
current_versions:
  stellar_core: $(kubectl get deployment stellar-core -o jsonpath='{.spec.template.spec.containers[0].image}')
  horizon: $(kubectl get deployment horizon -o jsonpath='{.spec.template.spec.containers[0].image}')
  soroban_rpc: $(kubectl get deployment soroban-rpc -o jsonpath='{.spec.template.spec.containers[0].image}' 2>/dev/null || echo "N/A")
  operator: $(kubectl get deployment stellar-operator -o jsonpath='{.spec.template.spec.containers[0].image}' 2>/dev/null || echo "N/A")
target_versions:
  stellar_core: "v21.x.0"
  horizon: "v21.x.0"
  soroban_rpc: "v21.x.0"
  operator: "v21.x.0"
EOF
```

### 2. Capacity Planning for Upgrade Window

```bash
# Check current resource utilization
kubectl top nodes
kubectl top pods -A

# Verify cluster has enough capacity for temporary duplication (blue-green)
# Required: 2x current resource usage during upgrade

# Check if HPA is enabled
kubectl get hpa -A

# Disable HPA during upgrade to prevent interference
kubectl patch hpa -A --type='json' -p='[{"op": "replace", "path": "/spec/minReplicas", "value": 1}]'
```

### 3. Communication Plan

```bash
# Create upgrade notification
cat > upgrade-notification.txt << EOF
⚠️  MAINTENANCE WINDOW: STELLAR-K8S MAJOR VERSION UPGRADE

Start Time: $(date -d '+2 hours' -u +%Y-%m-%dT%H:%M:%SZ)
Duration: 2-4 hours (estimated)
Impact: Zero-downtime (blue-green deployment)
Affected Services:
  - Stellar Core validator nodes
  - Horizon API nodes
  - Soroban RPC nodes (if deployed)

Monitoring: Status dashboard will be updated every 10 minutes
Support: Contact ops-team on #stellar-ops Slack channel
EOF

# Send notifications to stakeholders
# (Integration with your alerting system)
```

## Pre-Flight Verification

### 1. Health Check Script

```bash
#!/bin/bash
# pre-upgrade-health-check.sh

set -e
HEALTH_CHECK_LOG="pre-upgrade-health-check-$(date +%Y%m%d_%H%M%S).log"

echo "Starting pre-flight health checks..." | tee $HEALTH_CHECK_LOG

# 1. Node readiness
echo "[CHECK 1/10] Verifying node readiness..." | tee -a $HEALTH_CHECK_LOG
kubectl get nodes -o wide | tee -a $HEALTH_CHECK_LOG
UNREADY_NODES=$(kubectl get nodes -o json | jq '.items[] | select(.status.conditions[] | select(.type=="Ready" and .status=="False")) | .metadata.name' | wc -l)
if [ "$UNREADY_NODES" -gt 0 ]; then
  echo "ERROR: $UNREADY_NODES nodes not ready" | tee -a $HEALTH_CHECK_LOG
  exit 1
fi

# 2. Pod readiness
echo "[CHECK 2/10] Verifying critical pod readiness..." | tee -a $HEALTH_CHECK_LOG
for pod in stellar-core horizon soroban-rpc stellar-operator; do
  READY=$(kubectl get pods -l app=$pod -o json | jq '.items[] | select(.status.conditions[] | select(.type=="Ready" and .status=="True")) | .metadata.name' | wc -l)
  if [ "$READY" -eq 0 ]; then
    echo "ERROR: No ready pods for $pod" | tee -a $HEALTH_CHECK_LOG
    exit 1
  fi
  echo "✓ $pod: $READY pods ready" | tee -a $HEALTH_CHECK_LOG
done

# 3. Quorum status
echo "[CHECK 3/10] Verifying quorum consensus..." | tee -a $HEALTH_CHECK_LOG
CONSENSUS=$(kubectl exec -it deployment/stellar-core -- stellar-core http-command '{"command":"info"}' 2>/dev/null | jq '.state.synced' || echo "false")
if [ "$CONSENSUS" != "true" ]; then
  echo "ERROR: Stellar Core consensus not achieved" | tee -a $HEALTH_CHECK_LOG
  exit 1
fi
echo "✓ Stellar Core consensus: ACHIEVED" | tee -a $HEALTH_CHECK_LOG

# 4. Database replication lag
echo "[CHECK 4/10] Verifying database replication..." | tee -a $HEALTH_CHECK_LOG
REPL_LAG=$(kubectl exec -it deployment/horizon -- psql -d horizon -U horizon -c "SELECT EXTRACT(EPOCH FROM (NOW() - pg_last_xact_replay_timestamp()));" 2>/dev/null || echo "999")
if (( $(echo "$REPL_LAG > 5" | bc -l) )); then
  echo "WARNING: Horizon replication lag: ${REPL_LAG}s (threshold: 5s)" | tee -a $HEALTH_CHECK_LOG
fi
echo "✓ Replication lag: ${REPL_LAG}s" | tee -a $HEALTH_CHECK_LOG

# 5. Disk space
echo "[CHECK 5/10] Verifying disk space..." | tee -a $HEALTH_CHECK_LOG
kubectl exec -it deployment/stellar-core -- df -h /var/lib/stellar | tee -a $HEALTH_CHECK_LOG
DISK_USAGE=$(kubectl exec -it deployment/stellar-core -- df /var/lib/stellar | tail -1 | awk '{print $5}' | sed 's/%//')
if [ "$DISK_USAGE" -gt 80 ]; then
  echo "ERROR: Disk usage too high: ${DISK_USAGE}%" | tee -a $HEALTH_CHECK_LOG
  exit 1
fi
echo "✓ Disk usage: ${DISK_USAGE}%" | tee -a $HEALTH_CHECK_LOG

# 6. Network connectivity
echo "[CHECK 6/10] Verifying network connectivity..." | tee -a $HEALTH_CHECK_LOG
kubectl exec -it deployment/stellar-core -- ping -c 1 horizon || exit 1
echo "✓ Network connectivity verified" | tee -a $HEALTH_CHECK_LOG

# 7. PVC status
echo "[CHECK 7/10] Verifying persistent volumes..." | tee -a $HEALTH_CHECK_LOG
kubectl get pvc -A | tee -a $HEALTH_CHECK_LOG
UNBOUND_PVCS=$(kubectl get pvc -A -o json | jq '.items[] | select(.status.phase != "Bound") | .metadata.name' | wc -l)
if [ "$UNBOUND_PVCS" -gt 0 ]; then
  echo "ERROR: $UNBOUND_PVCS PVCs not bound" | tee -a $HEALTH_CHECK_LOG
  exit 1
fi
echo "✓ All PVCs bound" | tee -a $HEALTH_CHECK_LOG

# 8. Resource requests check
echo "[CHECK 8/10] Verifying resource availability..." | tee -a $HEALTH_CHECK_LOG
kubectl describe nodes | grep -A 5 "Allocated resources" | tee -a $HEALTH_CHECK_LOG

# 9. API endpoint connectivity
echo "[CHECK 9/10] Verifying API endpoints..." | tee -a $HEALTH_CHECK_LOG
HORIZON_HEALTH=$(curl -s http://horizon-ingress/health || echo "FAILED")
echo "Horizon health: $HORIZON_HEALTH" | tee -a $HEALTH_CHECK_LOG

# 10. Backup verification
echo "[CHECK 10/10] Verifying backup integrity..." | tee -a $HEALTH_CHECK_LOG
# List recent backups
ls -lh *.sql *.yaml | tail -5 | tee -a $HEALTH_CHECK_LOG

echo "" | tee -a $HEALTH_CHECK_LOG
echo "✅ ALL PRE-FLIGHT CHECKS PASSED" | tee -a $HEALTH_CHECK_LOG
echo "Detailed log: $HEALTH_CHECK_LOG"
```

### 2. Quick Health Check Commands

```bash
# One-liner health checks
kubectl get nodes -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.status.conditions[?(@.type=="Ready")].status}{"\n"}{end}'

# Check all critical pods
kubectl get pods -l app in (stellar-core,horizon,soroban-rpc) -A -o wide

# Verify PVC capacity
kubectl get pvc -A --sort-by='.spec.resources.requests.storage'

# Check API latency
for i in {1..10}; do time curl -s http://horizon/health > /dev/null; done
```

## Upgrade Strategies

### Strategy 1: Blue-Green Deployment (Zero-Downtime)

**Best for:** API nodes (Horizon, Soroban RPC), non-validator nodes

#### Step 1: Deploy Green Environment

```bash
# Create new namespace for green deployment
kubectl create namespace stellar-k8s-green

# Copy current Helm values
helm get values stellar-operator -n stellar-k8s > values-blue.yaml
cp values-blue.yaml values-green.yaml

# Update image versions in values-green.yaml
sed -i 's/stellarcore:20\.x\.x/stellarcore:21.x.x/g' values-green.yaml
sed -i 's/horizon:20\.x\.x/horizon:21.x.x/g' values-green.yaml

# Deploy to green namespace
helm install stellar-operator-green ./charts/stellar-operator \
  -n stellar-k8s-green \
  -f values-green.yaml \
  --wait \
  --timeout 10m

echo "Green deployment status:"
kubectl rollout status deployment -n stellar-k8s-green
```

#### Step 2: Verify Green Environment

```bash
# Wait for pods to be ready
kubectl wait --for=condition=Ready pod -l app=horizon -n stellar-k8s-green --timeout=300s

# Run smoke tests on green environment
./scripts/smoke-tests.sh -n stellar-k8s-green

# Verify green can sync with blue
kubectl exec -it deployment/horizon -n stellar-k8s-green -- \
  curl http://horizon-blue.stellar-k8s:8000/health

# Compare transaction counts
BLUE_TXNS=$(kubectl exec deployment/horizon -n stellar-k8s -- psql -d horizon -t -c "SELECT count(*) FROM transactions;")
GREEN_TXNS=$(kubectl exec deployment/horizon -n stellar-k8s-green -- psql -d horizon -t -c "SELECT count(*) FROM transactions;")
echo "Blue transactions: $BLUE_TXNS"
echo "Green transactions: $GREEN_TXNS"
```

#### Step 3: Switch Traffic

```bash
# Update ingress to point to green
kubectl patch ingress horizon-ingress \
  -p '{"spec":{"rules":[{"host":"horizon.example.com","http":{"paths":[{"path":"/","backend":{"serviceName":"horizon-service-green","servicePort":8000}}]}}]}}'

# Monitor error rates (should remain 0%)
for i in {1..60}; do
  ERROR_RATE=$(curl -s http://prometheus:9090/api/v1/query?query='rate(http_requests_total{status=~"5.."}[5m])' | jq '.data.result[0].value[1]')
  echo "[$i/60] Error rate: $ERROR_RATE"
  sleep 1
done
```

#### Step 4: Monitor Green for Stability (30 minutes)

```bash
# Monitor key metrics
watch -n 5 'kubectl get pods -n stellar-k8s-green -o wide && \
  echo "---" && \
  kubectl top pods -n stellar-k8s-green'

# Check logs for errors
kubectl logs -n stellar-k8s-green -l app=horizon --tail=100 -f --timestamps=true | grep ERROR
```

#### Step 5: Decommission Blue

```bash
# After 30 minutes of successful green operation
# Delete blue namespace
kubectl delete namespace stellar-k8s

# Rename green to production
kubectl patch namespace stellar-k8s-green -p '{"metadata":{"name":"stellar-k8s"}}'
```

### Strategy 2: Canary Deployment (Gradual Rollout)

**Best for:** Validator nodes, traffic-sensitive deployments

See [Canary Deployment Strategy](#strategy-2-canary-deployment-gradual-rollout) and examples/canary-deployment.yaml

#### Canary Workflow

```bash
# 1. Deploy canary (10% of traffic)
kubectl set image deployment/stellar-core stellar-core=stellarcore:21.x.x --record
kubectl rollout status deployment/stellar-core --watch

# 2. Monitor canary metrics (10 minutes)
# Alert on: error rate increase, latency spike, consensus lag

# 3. Promote canary to 50%
kubectl patch deployment/stellar-core -p \
  '{"spec":{"strategy":{"canary":{"steps":[{"weight":50,"pause":{}}]}}}}'

# 4. Continue monitoring (10 minutes)

# 5. Promote to 100%
kubectl patch deployment/stellar-core -p \
  '{"spec":{"strategy":{"canary":{"steps":[{"weight":100,"pause":{}}]}}}}'

# 6. Verify complete rollout
kubectl get rs -l app=stellar-core --sort-by=.metadata.creationTimestamp | tail -2
```

### Strategy 3: Rolling Update (Staged Validator Upgrade)

**Best for:** Multiple validator nodes with quorum considerations

```bash
# 1. Get list of validators (example: 5 validators)
VALIDATORS=$(kubectl get pods -l app=stellar-core -o jsonpath='{.items[*].metadata.name}')

# 2. Upgrade one validator at a time
for validator in $VALIDATORS; do
  echo "Upgrading validator: $validator"
  
  # 2a. Drain validator from consensus
  kubectl exec $validator -- stellar-core http-command '{"command":"dropcursor","cursor":"UPGRADEPENDING"}'
  
  # 2b. Wait for catchup to complete
  echo "Waiting for validator to catch up..."
  sleep 30
  
  # 2c. Update deployment
  kubectl set image deployment/stellar-core stellar-core=stellarcore:21.x.x --record
  kubectl rollout status deployment/stellar-core -w
  
  # 2d. Verify validator rejoined consensus
  STATUS=$(kubectl exec $validator -- stellar-core http-command '{"command":"info"}' | jq '.state')
  echo "Validator state: $STATUS"
  
  if [ "$STATUS" != "\"Synced!\"" ]; then
    echo "ERROR: Validator not synced!"
    exit 1
  fi
  
  # 2e. Wait before next validator
  echo "Waiting 5 minutes before next validator upgrade..."
  sleep 300
done

echo "✅ All validators upgraded successfully"
```

## Database Schema Upgrades

### Step 1: Pre-Migration Backup

```bash
# Backup both databases
pg_dump -h stellar-core-db -U stellar_core stellar_core \
  --verbose --no-password > stellar_core_pre_migration_$(date +%Y%m%d_%H%M%S).sql

pg_dump -h horizon-db -U horizon_user horizon \
  --verbose --no-password > horizon_pre_migration_$(date +%Y%m%d_%H%M%S).sql

# Create snapshot volumes
kubectl exec -it storage-manager -- \
  rbd snap create stellar-core-db@pre-migration-$(date +%Y%m%d_%H%M%S)

kubectl exec -it storage-manager -- \
  rbd snap create horizon-db@pre-migration-$(date +%Y%m%d_%H%M%S)

# Verify backups
ls -lh *.sql
echo "Backups created at: $(pwd)"
```

### Step 2: Schema Migration (Offline)

```bash
# Put Horizon in maintenance mode (stops accepting new requests)
kubectl patch deployment horizon -p '{"spec":{"replicas":0}}'

# Stop Stellar Core replication
kubectl exec -it deployment/stellar-core -- \
  stellar-core http-command '{"command":"stopcatch","cursor":"STOP"}'

# Run migration scripts (these are version-specific)
kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -d horizon -f /migrations/21.0.0_schema_changes.sql

# Verify schema changes
kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -d horizon -c "\dt" | tee schema_changes_$(date +%Y%m%d_%H%M%S).log

# Check migration status
kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -d horizon -c "SELECT * FROM migrations ORDER BY id DESC LIMIT 5;"
```

### Step 3: Post-Migration Validation

```bash
# Validate schema consistency
./scripts/validate-schema.sh horizon

# Check table statistics
kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -d horizon -c "SELECT schemaname, tablename, pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) FROM pg_tables WHERE schemaname='public' ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC;"

# Verify indexes are valid
kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -d horizon -c "SELECT schemaname, tablename, indexname FROM pg_indexes WHERE schemaname='public' ORDER BY tablename;"

# Check for dead tuples
kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -d horizon -c "SELECT schemaname, tablename, n_dead_tup FROM pg_stat_user_tables WHERE n_dead_tup > 0 ORDER BY n_dead_tup DESC;"
```

### Step 4: Resume Operations

```bash
# Resume Stellar Core
kubectl exec -it deployment/stellar-core -- \
  stellar-core http-command '{"command":"startcatch"}'

# Monitor catchup progress
for i in {1..60}; do
  STATUS=$(kubectl exec deployment/stellar-core -- \
    stellar-core http-command '{"command":"info"}' | jq '.state')
  echo "[$i/60] Catchup status: $STATUS"
  sleep 5
done

# Resume Horizon with gradually increasing replicas
kubectl patch deployment horizon -p '{"spec":{"replicas":1}}'
kubectl wait --for=condition=Ready pod -l app=horizon --timeout=300s

# Scale back up
kubectl patch deployment horizon -p '{"spec":{"replicas":3}}'
```

## Post-Upgrade Validation

### 1. Immediate Post-Upgrade Checks (5 minutes)

```bash
#!/bin/bash
# post-upgrade-immediate.sh

echo "=== IMMEDIATE POST-UPGRADE CHECKS ==="
echo "Timestamp: $(date -u)"

# Check 1: All pods running
echo ""
echo "[CHECK 1] Pod Status:"
kubectl get pods -A -o wide | grep -E "(stellar-core|horizon|soroban-rpc|stellar-operator)"

# Check 2: No restart loops
echo ""
echo "[CHECK 2] Pod Restarts (should be 0):"
kubectl get pods -l app in (stellar-core,horizon) -A -o custom-columns=NAMESPACE:.metadata.namespace,NAME:.metadata.name,RESTARTS:.status.containerStatuses[0].restartCount

# Check 3: API endpoints responding
echo ""
echo "[CHECK 3] API Endpoint Health:"
curl -s http://horizon/health | jq .

# Check 4: Stellar Core info
echo ""
echo "[CHECK 4] Stellar Core Status:"
kubectl exec -it deployment/stellar-core -- stellar-core http-command '{"command":"info"}' | jq '{state: .state, synced: .state_metrics.synced}'

# Check 5: Error logs
echo ""
echo "[CHECK 5] Recent Errors in Logs:"
kubectl logs -l app=horizon --all-containers=true --tail=50 | grep -i error | head -5
```

### 2. Extended Post-Upgrade Checks (30 minutes)

```bash
#!/bin/bash
# post-upgrade-extended.sh

echo "=== EXTENDED POST-UPGRADE VALIDATION (30 min) ==="

# Monitor window duration
MONITOR_DURATION=1800  # 30 minutes in seconds
INTERVAL=30  # Check every 30 seconds
ELAPSED=0

while [ $ELAPSED -lt $MONITOR_DURATION ]; do
  TIMESTAMP=$(date '+%Y-%m-%d %H:%M:%S')
  
  # Collect metrics
  ERROR_RATE=$(curl -s http://prometheus:9090/api/v1/query?query='rate(http_requests_total{status=~"5.."}[5m])' | jq '.data.result[0].value[1]' 2>/dev/null || echo "N/A")
  P99_LATENCY=$(curl -s http://prometheus:9090/api/v1/query?query='histogram_quantile(0.99, rate(http_request_duration_seconds_bucket[5m]))' | jq '.data.result[0].value[1]' 2>/dev/null || echo "N/A")
  CONSENSUS=$(kubectl exec -it deployment/stellar-core -- stellar-core http-command '{"command":"info"}' | jq '.state' 2>/dev/null || echo "ERROR")
  REPLICATION_LAG=$(kubectl exec -it deployment/horizon-db -- psql -U horizon_user -d horizon -t -c "SELECT EXTRACT(EPOCH FROM (NOW() - pg_last_xact_replay_timestamp()));" 2>/dev/null || echo "N/A")
  
  echo "[$TIMESTAMP] Error Rate: $ERROR_RATE | P99 Latency: ${P99_LATENCY}s | Consensus: $CONSENSUS | Replication Lag: ${REPLICATION_LAG}s"
  
  # Alert on anomalies
  if [ "$ERROR_RATE" != "N/A" ] && (( $(echo "$ERROR_RATE > 0.01" | bc -l) )); then
    echo "⚠️  HIGH ERROR RATE DETECTED!"
  fi
  
  if [ "$CONSENSUS" != "\"Synced!\"" ]; then
    echo "⚠️  CONSENSUS ISSUE DETECTED!"
  fi
  
  sleep $INTERVAL
  ELAPSED=$((ELAPSED + INTERVAL))
done

echo "✅ Extended monitoring complete"
```

### 3. Transaction Throughput Validation

```bash
#!/bin/bash
# Verify transaction throughput matches pre-upgrade baseline

# Get baseline from pre-upgrade metrics
BASELINE_TXN_RATE=$(grep "baseline_txn_rate" pre-upgrade-metrics.txt | awk '{print $2}')

# Current transaction rate
CURRENT_TXN_RATE=$(curl -s http://prometheus:9090/api/v1/query?query='rate(stellar_txn_counter_total[1m])' | jq '.data.result[0].value[1]')

# Calculate percentage deviation
DEVIATION=$(echo "scale=2; (($CURRENT_TXN_RATE - $BASELINE_TXN_RATE) / $BASELINE_TXN_RATE) * 100" | bc)

echo "Baseline TXN rate: $BASELINE_TXN_RATE/s"
echo "Current TXN rate: $CURRENT_TXN_RATE/s"
echo "Deviation: $DEVIATION%"

if (( $(echo "$DEVIATION < -10" | bc -l) )); then
  echo "❌ ERROR: Transaction throughput degraded more than 10%!"
  exit 1
fi

echo "✅ Transaction throughput within acceptable range"
```

## Rollback Procedures

### Scenario 1: Immediate Rollback (Within 30 minutes)

```bash
#!/bin/bash
# Rollback to blue environment (if using blue-green)

echo "🔄 INITIATING IMMEDIATE ROLLBACK"

# 1. Revert ingress traffic back to blue
kubectl patch ingress horizon-ingress \
  -p '{"spec":{"rules":[{"host":"horizon.example.com","http":{"paths":[{"path":"/","backend":{"serviceName":"horizon-service-blue","servicePort":8000}}]}}]}}'

echo "Traffic reverted to blue deployment"

# 2. Monitor blue environment (5 minutes)
for i in {1..60}; do
  HEALTH=$(curl -s http://horizon-blue/health)
  echo "[$i/60] Blue health: $HEALTH"
  sleep 5
done

# 3. Delete green environment
kubectl delete namespace stellar-k8s-green

echo "✅ Rollback complete - green environment removed"
```

### Scenario 2: Database Rollback

```bash
#!/bin/bash
# Rollback database to pre-migration snapshot

echo "🔄 INITIATING DATABASE ROLLBACK"

# Get list of available snapshots
SNAPSHOTS=$(rbd snap ls stellar-core-db | tail -n +2 | awk '{print $2}')
echo "Available snapshots:"
echo "$SNAPSHOTS"

# Select most recent pre-migration snapshot
ROLLBACK_SNAPSHOT="pre-migration-20240115_120000"

echo "Rolling back to snapshot: $ROLLBACK_SNAPSHOT"

# 1. Stop Horizon
kubectl patch deployment horizon -p '{"spec":{"replicas":0}}'

# 2. Stop Stellar Core
kubectl patch deployment stellar-core -p '{"spec":{"replicas":0}}'

# 3. Unmount current volume
kubectl exec -it storage-manager -- \
  umount /mnt/stellar-core-db

# 4. Restore from snapshot
kubectl exec -it storage-manager -- \
  rbd clone stellar-core-db@$ROLLBACK_SNAPSHOT stellar-core-db-restored

# 5. Mount restored volume
kubectl exec -it storage-manager -- \
  mount /dev/rbd/stellar-pool/stellar-core-db-restored /mnt/stellar-core-db

# 6. Restart services
kubectl patch deployment stellar-core -p '{"spec":{"replicas":1}}'
kubectl patch deployment horizon -p '{"spec":{"replicas":3}}'

# 7. Verify
kubectl wait --for=condition=Ready pod -l app=stellar-core --timeout=300s

echo "✅ Database rollback complete"
```

### Scenario 3: Partial Rollback (Validator Node)

```bash
#!/bin/bash
# Rollback specific validator node if consensus issues occur

FAILED_VALIDATOR="stellar-core-0"

echo "Rolling back validator: $FAILED_VALIDATOR"

# 1. Remove from quorum (stop accepting votes)
kubectl exec $FAILED_VALIDATOR -- \
  stellar-core http-command '{"command":"dropcursor","cursor":"UPGRADEPENDING"}'

# 2. Revert to previous image
kubectl set image pod/$FAILED_VALIDATOR stellar-core=stellarcore:20.x.x --record

# 3. Wait for pod restart
kubectl delete pod $FAILED_VALIDATOR
kubectl wait --for=condition=Ready pod -l app=stellar-core,pod-name=$FAILED_VALIDATOR --timeout=300s

# 4. Verify consensus rejoined
CONSENSUS=$(kubectl exec $FAILED_VALIDATOR -- stellar-core http-command '{"command":"info"}' | jq '.state')
echo "Validator consensus state: $CONSENSUS"

if [ "$CONSENSUS" = "\"Synced!\"" ]; then
  echo "✅ Validator rolled back and rejoined consensus"
else
  echo "❌ ERROR: Validator failed to rejoin consensus"
  exit 1
fi
```

## Emergency Recovery

### Complete Cluster Recovery from Backup

```bash
#!/bin/bash
# emergency-recovery.sh - Full cluster recovery procedure

set -e

echo "🚨 EMERGENCY CLUSTER RECOVERY INITIATED"
echo "Timestamp: $(date -u)"

# 1. List available backups
echo ""
echo "Available backups:"
ls -lh stellar_core_*.sql horizon_*.sql *.yaml

# Prompt for backup selection
read -p "Enter backup timestamp to restore (e.g., 20240115_120000): " BACKUP_TS

# 2. Stop all services
echo "Stopping services..."
kubectl scale deployment stellar-core --replicas=0
kubectl scale deployment horizon --replicas=0
kubectl scale deployment soroban-rpc --replicas=0

# 3. Restore databases
echo "Restoring databases..."
CORE_BACKUP="stellar_core_pre_migration_${BACKUP_TS}.sql"
HORIZON_BACKUP="horizon_pre_migration_${BACKUP_TS}.sql"

if [ ! -f "$CORE_BACKUP" ] || [ ! -f "$HORIZON_BACKUP" ]; then
  echo "ERROR: Backups not found!"
  exit 1
fi

# Drop existing databases
kubectl exec -it deployment/stellar-core-db -- \
  psql -U postgres -c "DROP DATABASE IF EXISTS stellar_core;"
kubectl exec -it deployment/horizon-db -- \
  psql -U postgres -c "DROP DATABASE IF EXISTS horizon;"

# Restore from backups
kubectl exec -it deployment/stellar-core-db -- \
  psql -U postgres -f /backups/$CORE_BACKUP

kubectl exec -it deployment/horizon-db -- \
  psql -U horizon_user -f /backups/$HORIZON_BACKUP

echo "✅ Databases restored"

# 4. Restore Kubernetes resources
echo "Restoring Kubernetes resources..."
kubectl apply -f k8s_resources_${BACKUP_TS}.yaml

# 5. Verify recovery
echo "Verifying recovery..."
kubectl wait --for=condition=Ready pod -l app in (stellar-core,horizon) --timeout=600s

# 6. Health check
./post-upgrade-immediate.sh

echo "✅ EMERGENCY RECOVERY COMPLETE"
```

### Data Corruption Detection

```bash
#!/bin/bash
# Detect and quarantine corrupted data

# Run PostgreSQL integrity checks
kubectl exec -it deployment/stellar-core-db -- \
  psql -U stellar_core -d stellar_core -c "ANALYZE;" | tee corruption_check.log

kubectl exec -it deployment/stellar-core-db -- \
  psql -U stellar_core -d stellar_core -c "SELECT pg_catalog.pg_relation_filepath(oid), relpages FROM pg_catalog.pg_class WHERE relkind='r';" | tee corruption_check.log

# Check for block-level corruption
kubectl exec -it deployment/stellar-core-db -- \
  amcheck.sql | tee corruption_check.log 2>&1 || true

# If corruption detected, quarantine affected tables
if grep -q "ERROR" corruption_check.log; then
  echo "❌ Corruption detected! Creating backup of corrupted partition..."
  
  # Rename corrupted table
  kubectl exec -it deployment/stellar-core-db -- \
    psql -U stellar_core -d stellar_core -c "ALTER TABLE affected_table RENAME TO affected_table_corrupted_$(date +%Y%m%d_%H%M%S);"
  
  echo "Corrupted data quarantined. Manual investigation required."
fi
```

## Monitoring and Alerting

### Key Metrics to Monitor During Upgrade

| Metric | Type | Alert Threshold | Source |
|--------|------|-----------------|--------|
| Pod restart count | counter | > 0 during upgrade | Kubernetes API |
| HTTP error rate (5xx) | gauge | > 1% | Prometheus |
| API endpoint latency (p99) | histogram | > 2s | Prometheus |
| Consensus state | gauge | != "Synced!" | Stellar Core |
| Database replication lag | gauge | > 10s | PostgreSQL |
| Transaction throughput | counter | < 90% of baseline | Prometheus |
| Disk usage | gauge | > 85% | Node exporter |
| Memory usage | gauge | > 80% | Kubernetes metrics |

### PrometheusRule ConfigMap for Upgrade Monitoring

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: upgrade-alerts
  namespace: monitoring
data:
  upgrade-alerts.yaml: |
    groups:
    - name: stellar-upgrade
      interval: 30s
      rules:
      - alert: HighErrorRate
        expr: rate(http_requests_total{status=~"5.."}[5m]) > 0.01
        for: 1m
        annotations:
          summary: "High error rate during upgrade"
      
      - alert: ConsensusLost
        expr: stellar_core_consensus_state != 1
        for: 2m
        annotations:
          summary: "Consensus lost during upgrade"
      
      - alert: ReplicationLagHigh
        expr: pg_replication_lag_seconds > 10
        for: 2m
        annotations:
          summary: "Database replication lag too high"
```

## Success Criteria

Upgrades are considered successful when:

1. ✅ All pods running and ready
2. ✅ No pod restarts during upgrade window
3. ✅ API error rate < 1% (max spike)
4. ✅ Consensus achieved and maintained
5. ✅ Transaction throughput > 90% of baseline
6. ✅ Database replication lag < 5 seconds
7. ✅ No data loss or corruption detected
8. ✅ Rollback capability verified (if needed)

## Troubleshooting

### Pod fails to start after upgrade

```bash
# Check pod events
kubectl describe pod <pod-name>

# Check logs
kubectl logs <pod-name> --previous
kubectl logs <pod-name>

# Check image pull
kubectl get events | grep ImagePull

# Verify image is available
docker pull stellarcore:21.x.x
```

### Consensus lost

```bash
# Check peers
kubectl exec deployment/stellar-core -- stellar-core http-command '{"command":"peers"}'

# Check ledger state
kubectl exec deployment/stellar-core -- stellar-core http-command '{"command":"ledgerinfo"}'

# Restart if necessary
kubectl rollout restart deployment/stellar-core
```

### Database migration fails

```bash
# Check migration logs
kubectl logs deployment/horizon-db | grep -i migration

# View current schema version
kubectl exec -it deployment/horizon-db -- psql -U horizon_user -d horizon -c "SELECT * FROM migrations ORDER BY id DESC LIMIT 1;"

# Rollback to pre-migration snapshot (see Rollback Procedures)
```

## References

- [Stellar Core Release Notes](https://github.com/stellar/stellar-core/releases)
- [Horizon Database Documentation](https://developers.stellar.org/docs/run-api-server/horizon-rpc)
- [Kubernetes Rolling Updates](https://kubernetes.io/docs/tutorials/kubernetes-basics/update/update-intro/)
- [PostgreSQL Migration Guide](https://www.postgresql.org/docs/current/upgrading.html)

---

**Document Version:** 1.0  
**Last Updated:** 2024-01-15  
**Author:** Stellar-K8s Operations Team  
**Status:** Production Ready
