---
title: Multi-Tenancy Platform - Migration Guide
epic_id: 870
status: draft
created: 2026-06-02
---

# Migration Guide: Single-Tenant to Multi-Tenant

## Overview

This guide helps existing Stellar-K8s users migrate from single-tenant deployments to the new multi-tenancy platform. The migration is designed to be **non-breaking** and can be done gradually.

## Pre-Migration Checklist

Before starting the migration, ensure you have:

- [ ] Stellar-K8s operator v2.0.0+ installed
- [ ] Kubernetes 1.28+ cluster
- [ ] Prometheus and Grafana installed
- [ ] OIDC provider configured (for portal access)
- [ ] Backup of all StellarNode resources (`kubectl get stellarnodes -A -o yaml > backup.yaml`)
- [ ] List of existing namespaces with Stellar resources
- [ ] Cost rates defined (CPU/memory/storage pricing)

## Migration Phases

### Phase 1: Enable Multi-Tenancy (Non-Breaking) ✅

This phase enables the tenant controller without affecting existing resources.

**1. Upgrade Helm Chart**
```bash
helm repo update
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --version 2.0.0 \
  --set tenantController.enabled=true \
  --set tenantController.dryRun=true \  # Read-only mode
  --reuse-values
```

**2. Verify Controller Started**
```bash
kubectl logs -n stellar-system deployment/stellar-operator -f | grep "tenant_controller"
```

Expected output:
```
INFO stellar_operator::controller::tenant_controller: Tenant controller started (dry-run mode)
```

**3. Verify Existing Nodes Still Work**
```bash
kubectl get stellarnodes -A
# All nodes should show Ready status
```

✅ **At this point, nothing has changed for existing nodes. They continue to operate normally.**

---

### Phase 2: Create Tenant Resources ✅

Map existing namespaces to Tenant CRDs.

**1. Identify Existing Stellar Namespaces**
```bash
kubectl get namespaces -l app.kubernetes.io/managed-by=stellar-operator
```

**2. Create Tenant for Each Namespace**

For each existing namespace, create a corresponding Tenant resource:

```bash
cat <<EOF | kubectl apply -f -
apiVersion: stellar.org/v1alpha1
kind: Tenant
metadata:
  name: existing-team
spec:
  tenantId: existing-team
  displayName: "Existing Team (Migrated)"
  namespace: existing-namespace  # Your existing namespace
  
  # Set quotas based on current usage
  quota:
    hard:
      cpu: "16"      # Set to current usage + headroom
      memory: "64Gi"
      storage: "1Ti"
      persistentvolumeclaims: "20"
  
  suspended: false
  cleanupOnDelete: false  # Important: don't delete namespace on Tenant deletion
  
  contacts:
    adminEmail: "admin@team.com"
EOF
```

**3. Label Existing Namespaces**
```bash
kubectl label namespace existing-namespace \
  tenant.stellar.org/id=existing-team
```

**4. Verify Tenant Status**
```bash
kubectl get tenant existing-team -o yaml
```

Check that `status.phase` is `Active` and all resources are created.

---

### Phase 3: Enable Quota Enforcement (Gradual) ⚠️

**WARNING**: This phase enforces quotas on existing namespaces. Start with generous quotas.

**1. Apply ResourceQuota to Tenant Namespace**

The Tenant controller will create ResourceQuotas automatically. Verify:

```bash
kubectl get resourcequota -n existing-namespace
```

Expected output:
```
NAME                     AGE   REQUEST
tenant-existing-team     1m    cpu: 0/16, memory: 0/64Gi
```

**2. Verify Current Usage is Below Quota**

```bash
kubectl describe resourcequota -n existing-namespace tenant-existing-team
```

If current usage exceeds quota, **increase the quota** before proceeding:

```bash
kubectl patch tenant existing-team --type=merge -p '
spec:
  quota:
    hard:
      cpu: "32"  # Increased
'
```

**3. Enable Webhook Validation**

```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set webhook.tenantValidation.enabled=true \
  --reuse-values
```

**4. Test Quota Enforcement**

Try creating a node that would exceed quota:

```bash
kubectl apply -f test-node-exceeding-quota.yaml
```

Expected: Admission denied with clear error message.

---

### Phase 4: Enable Network Isolation 🔒

Apply NetworkPolicies to prevent cross-tenant traffic.

**1. Review Current Network Policies**

```bash
kubectl get networkpolicies -A
```

**2. Enable Tenant NetworkPolicies**

```bash
kubectl patch tenant existing-team --type=merge -p '
spec:
  network:
    labelKey: tenant.stellar.org/id
    labelValue: existing-team
'
```

The Tenant controller will create NetworkPolicies automatically.

**3. Verify NetworkPolicies Applied**

```bash
kubectl get networkpolicy -n existing-namespace
```

Expected output:
```
NAME                        POD-SELECTOR   AGE
tenant-isolation            <all pods>     30s
```

**4. Test Cross-Tenant Access Blocked**

If you have multiple tenants, verify pods cannot reach each other:

```bash
# From tenant-a pod
kubectl exec -n tenant-a-namespace <pod> -- curl http://service.tenant-b-namespace
# Should timeout or be denied
```

---

### Phase 5: Enable Cost Tracking 💰

Start collecting usage data for billing/chargeback.

**1. Deploy Cost Collector CronJob**

```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set costCollector.enabled=true \
  --set costCollector.costRates.cpuPerCoreHour=0.10 \
  --set costCollector.costRates.memoryPerGiBHour=0.02 \
  --set costCollector.costRates.storagePerGiBHour=0.0001 \
  --reuse-values
```

**2. Manually Trigger Cost Collector (First Run)**

```bash
kubectl create job -n stellar-system cost-collector-manual \
  --from=cronjob/tenant-cost-collector
```

**3. Verify TenantUsage Created**

```bash
kubectl get tenantusage -n stellar-system
```

Expected output:
```
NAME                       TENANT-ID       TOTAL-COST   AGE
existing-team-2026-06      existing-team   532.80       1m
```

**4. View Cost Details**

```bash
kubectl get tenantusage existing-team-2026-06 -o yaml
```

---

### Phase 6: Deploy Self-Service Portal 🖥️

Enable the web UI for tenant self-service.

**1. Configure OIDC Provider**

Create secret with OIDC credentials:

```bash
kubectl create secret generic portal-oidc \
  -n stellar-system \
  --from-literal=client-id=<your-client-id> \
  --from-literal=client-secret=<your-client-secret>
```

**2. Deploy Portal**

```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set portal.enabled=true \
  --set portal.replicas=2 \
  --set portal.oidc.issuerUrl=https://accounts.google.com \
  --set portal.ingress.enabled=true \
  --set portal.ingress.host=stellar-portal.example.com \
  --reuse-values
```

**3. Access Portal**

```bash
kubectl get ingress -n stellar-system stellar-portal-ui
# Navigate to the URL in your browser
```

**4. Login and Verify**

- Login with OIDC credentials
- Verify you can see your tenant dashboard
- Verify quota usage is displayed correctly

---

### Phase 7: Enable Grafana Dashboards 📊

Provision per-tenant Grafana dashboards.

**1. Install Grafana Operator (if not already installed)**

```bash
kubectl apply -f https://raw.githubusercontent.com/grafana-operator/grafana-operator/v5.0.0/deploy/manifests/cluster-scoped/grafana-operator-deployment.yaml
```

**2. Enable Dashboard Provisioning**

```bash
kubectl patch tenant existing-team --type=merge -p '
spec:
  dashboard:
    enabled: true
    grafanaNamespace: monitoring
'
```

**3. Verify Dashboard Created**

```bash
kubectl get grafanadashboard -n monitoring
```

Expected output:
```
NAME                           AGE
tenant-existing-team-overview  1m
```

**4. Access Dashboard in Grafana**

The dashboard URL is written to Tenant status:

```bash
kubectl get tenant existing-team -o jsonpath='{.status.dashboardUrl}'
```

---

## Migration Validation

After completing all phases, validate the migration:

### ✅ Functional Validation

```bash
# 1. All existing nodes still running
kubectl get stellarnodes -A --field-selector status.phase=Ready
# Should show all nodes

# 2. All tenants are Active
kubectl get tenants -o custom-columns=NAME:.metadata.name,PHASE:.status.phase
# All should be "Active"

# 3. Quotas are enforced
kubectl describe resourcequota -n <tenant-namespace>
# Should show used vs hard limits

# 4. NetworkPolicies exist
kubectl get networkpolicies -A
# Should show policies per tenant namespace

# 5. Cost data is collected
kubectl get tenantusage -n stellar-system
# Should show usage records

# 6. Portal is accessible
curl -I https://stellar-portal.example.com
# Should return 200 OK
```

### ✅ Security Validation

```bash
# 1. Tenants cannot access other tenants' namespaces
kubectl auth can-i get pods -n other-tenant-namespace \
  --as=system:serviceaccount:tenant-a-namespace:default
# Should be "no"

# 2. Tenants cannot exceed quotas
# Try creating node exceeding quota - should fail

# 3. Tenants cannot bypass network policies
# Try curl from tenant-a to tenant-b - should timeout

# 4. Audit logs capture tenant operations
kubectl logs -n stellar-system deployment/stellar-operator | grep audit
# Should show tenant context in logs
```

### ✅ Performance Validation

```bash
# 1. Controller reconciliation time
kubectl get --raw /metrics | grep stellar_tenant_reconcile_duration_seconds
# P95 should be < 5s

# 2. Webhook latency
kubectl get --raw /metrics | grep stellar_webhook_duration_seconds
# P99 should be < 100ms

# 3. Portal API latency
kubectl get --raw /metrics | grep http_request_duration_seconds
# P95 should be < 500ms
```

---

## Rollback Procedure

If you need to rollback:

### Rollback Phase 7 (Grafana Dashboards)
```bash
kubectl patch tenant <name> --type=merge -p '{"spec":{"dashboard":{"enabled":false}}}'
```

### Rollback Phase 6 (Portal)
```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set portal.enabled=false \
  --reuse-values
```

### Rollback Phase 5 (Cost Tracking)
```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set costCollector.enabled=false \
  --reuse-values
```

### Rollback Phase 4 (Network Isolation)
```bash
kubectl delete networkpolicy -n <tenant-namespace> tenant-isolation
```

### Rollback Phase 3 (Quota Enforcement)
```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set webhook.tenantValidation.enabled=false \
  --reuse-values
```

### Rollback Phase 2 (Tenant Resources)
```bash
kubectl delete tenant <tenant-name>
# Important: Set cleanupOnDelete: false before deleting!
```

### Rollback Phase 1 (Tenant Controller)
```bash
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --set tenantController.enabled=false \
  --reuse-values
```

---

## Troubleshooting

### Issue: Tenant stuck in "Provisioning" phase

**Diagnosis**:
```bash
kubectl describe tenant <name>
# Check conditions for errors
```

**Solution**:
- Check operator logs: `kubectl logs -n stellar-system deployment/stellar-operator -f`
- Verify namespace exists: `kubectl get namespace <tenant-namespace>`
- Verify RBAC: Controller ServiceAccount needs permissions

---

### Issue: Quota enforcement not working

**Diagnosis**:
```bash
kubectl get resourcequota -n <tenant-namespace>
# Check if ResourceQuota exists
```

**Solution**:
- Verify Tenant controller is running
- Check if quota is set in Tenant spec
- Verify webhook is enabled and reachable

---

### Issue: Cross-tenant traffic not blocked

**Diagnosis**:
```bash
kubectl get networkpolicy -n <tenant-namespace>
# Check if NetworkPolicy exists
```

**Solution**:
- Verify CNI plugin supports NetworkPolicies (Calico, Cilium, Weave)
- Check namespace labels: `kubectl get namespace <name> --show-labels`
- Verify pods have correct labels

---

### Issue: Cost collector fails

**Diagnosis**:
```bash
kubectl logs -n stellar-system job/tenant-cost-collector-<timestamp>
```

**Solution**:
- Verify Prometheus is accessible: `curl http://prometheus.monitoring:9090/-/healthy`
- Check cost rates are configured
- Verify ServiceAccount has permissions to create TenantUsage

---

### Issue: Portal login fails

**Diagnosis**:
```bash
kubectl logs -n stellar-system deployment/stellar-portal-api -f
```

**Solution**:
- Verify OIDC client ID/secret are correct
- Check OIDC issuer URL is reachable
- Verify redirect URI is configured in OIDC provider
- Check browser console for errors

---

## FAQ

### Q: Will existing StellarNodes be affected by the migration?

**A:** No. The migration is designed to be non-breaking. Existing StellarNodes continue to operate normally. They will be gradually associated with Tenant resources, but their runtime behavior does not change.

### Q: Can I run single-tenant and multi-tenant deployments in the same cluster?

**A:** Yes. Namespaces without a corresponding Tenant resource continue to operate in "legacy" mode. You can migrate namespaces one at a time.

### Q: What happens if I delete a Tenant resource?

**A:** It depends on `spec.cleanupOnDelete`:
- `cleanupOnDelete: true` → Namespace and all resources are deleted
- `cleanupOnDelete: false` → Namespace is retained, only Tenant CRD is removed

For migration, always set `cleanupOnDelete: false` initially.

### Q: How do I calculate appropriate quotas for existing namespaces?

**A:** Use Prometheus to query historical usage:

```promql
# Max CPU usage (cores)
max_over_time(
  sum(rate(container_cpu_usage_seconds_total{namespace="your-namespace"}[5m]))[30d:1h]
)

# Max memory usage (GiB)
max_over_time(
  sum(container_memory_working_set_bytes{namespace="your-namespace"})[30d:1h]
) / (1024^3)
```

Set quotas to 1.5x historical max to allow for growth.

### Q: Can I use different cost rates per tenant?

**A:** Yes. Set `spec.billing.costRates` in the Tenant resource to override global defaults.

### Q: How do I migrate StellarNode labels to include tenant ownership?

**A:** The Tenant controller can automatically propagate labels if `spec.labelPropagation` is configured. Alternatively, manually patch existing nodes:

```bash
kubectl patch stellarnode <name> -n <namespace> --type=merge -p '
metadata:
  labels:
    tenant.stellar.org/id: <tenant-id>
'
```

### Q: Is multi-cluster tenancy supported?

**A:** Not in v1. Each Kubernetes cluster runs its own multi-tenancy platform. Multi-cluster spanning tenants may be added in v2.

---

## Post-Migration Checklist

After completing the migration:

- [ ] All existing nodes are running and healthy
- [ ] All namespaces have corresponding Tenant resources
- [ ] Quotas are set and enforced
- [ ] NetworkPolicies are applied and tested
- [ ] Cost tracking is generating TenantUsage records
- [ ] Portal is accessible and functional
- [ ] Grafana dashboards are provisioned
- [ ] RBAC is configured (tenant admins, viewers)
- [ ] Documentation is updated for team
- [ ] Runbooks are created for common operations
- [ ] Monitoring alerts are configured
- [ ] Backup/restore procedures are tested

---

## Need Help?

- **Documentation**: https://github.com/OtowoOrg/Stellar-K8s/tree/main/docs
- **Issues**: https://github.com/OtowoOrg/Stellar-K8s/issues
- **Discussions**: https://github.com/OtowoOrg/Stellar-K8s/discussions
- **Slack**: #stellar-k8s on Kubernetes Slack

---

**Last Updated**: 2026-06-02  
**Applies to**: Stellar-K8s v2.0.0+
