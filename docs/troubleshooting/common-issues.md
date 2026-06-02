# Common Issues and Solutions

Solutions to frequently encountered problems when deploying and managing Stellar-K8s.

## Installation Issues

### Issue 1: CRDs Not Creating

**Symptoms:**
```
Error: unable to recognize "validator.yaml": no matches for kind "StellarValidator"
```

**Root Cause:** Custom Resource Definitions (CRDs) are not installed.

**Solution:**
```bash
# Verify CRDs exist
kubectl get crds | grep stellar

# If missing, reinstall Stellar-K8s
helm upgrade --install stellar-k8s stellar-k8s/stellar-k8s \
  --namespace stellar-system \
  --create-namespace
```

---

### Issue 2: Operator Pod CrashLoopBackOff

**Symptoms:**
```
stellar-operator-xxx   0/1   CrashLoopBackOff
```

**Root Cause:** Insufficient RBAC permissions or image pull errors.

**Solution:**
```bash
# Check operator logs
kubectl logs -n stellar-system deployment/stellar-operator

# Common fixes:
# 1. Verify ServiceAccount has correct permissions
kubectl get clusterrolebinding | grep stellar

# 2. Check image pull secrets
kubectl describe pod -n stellar-system <operator-pod-name>
```

---

### Issue 3: ImagePullBackOff

**Symptoms:**
```
validator-0   0/1   ImagePullBackOff
```

**Root Cause:** Image doesn't exist or registry authentication failed.

**Solution:**
```bash
# Check image configuration
kubectl describe pod validator-0 -n stellar

# Verify image exists
docker pull stellar/stellar-core:latest

# Add imagePullSecrets if using private registry
kubectl create secret docker-registry regcred \
  --docker-server=<registry> \
  --docker-username=<user> \
  --docker-password=<pass> \
  -n stellar
```

---

## Deployment Issues

### Issue 4: PVC Pending

**Symptoms:**
```
data-validator-0   Pending
```

**Root Cause:** No StorageClass available or insufficient storage.

**Solution:**
```bash
# Check StorageClasses
kubectl get storageclasses

# If none exist, create one or specify an existing class
kubectl patch stellarvalidator validator \
  --type='merge' \
  -p '{"spec":{"storage":{"storageClassName":"your-storage-class"}}}'
```

---

### Issue 5: Pod Stuck in Pending

**Symptoms:**
Pod never starts, stays in Pending state.

**Root Cause:** Insufficient cluster resources or node selector mismatch.

**Solution:**
```bash
# Check pod events
kubectl describe pod validator-0 -n stellar

# Common reasons:
# 1. Insufficient CPU/memory
kubectl top nodes

# 2. Node selector doesn't match any nodes
kubectl get nodes --show-labels

# 3. Pod affinity/anti-affinity rules too strict
kubectl get stellarvalidator validator -o yaml | grep -A10 affinity
```

---

### Issue 6: Service Not Accessible

**Symptoms:**
Cannot access validator through LoadBalancer service.

**Root Cause:** LoadBalancer not provisioning or firewall rules blocking traffic.

**Solution:**
```bash
# Check service status
kubectl get svc -n stellar

# Verify external IP is assigned
kubectl describe svc validator-service -n stellar

# For cloud providers, check load balancer in cloud console
# For on-prem, ensure MetalLB or similar is installed
```

---

## Runtime Issues

### Issue 7: Validator Not Syncing

**Symptoms:**
```
stellar_core_ledger_age_seconds > 60
```

**Root Cause:** Network connectivity issues or peer problems.

**Solution:**
```bash
# Check peer connections
kubectl exec validator-0 -n stellar -- stellar-core --c peers

# Verify history archive access
kubectl exec validator-0 -n stellar -- \
  curl -I https://history.stellar.org/prd/core-live/core_live_001/

# Check catchup status
kubectl logs validator-0 -n stellar | grep -i catchup

# Force catchup if needed
kubectl exec validator-0 -n stellar -- \
  stellar-core --c 'catchup complete'
```

---

### Issue 8: High Memory Usage

**Symptoms:**
Pod OOMKilled or high memory utilization.

**Root Cause:** Insufficient memory allocation or memory leak.

**Solution:**
```bash
# Check current usage
kubectl top pod validator-0 -n stellar

# Increase memory limits
kubectl patch stellarvalidator validator \
  --type='merge' \
  -p '{"spec":{"resources":{"limits":{"memory":"32Gi"}}}}'

# Check for memory leaks in logs
kubectl logs validator-0 -n stellar | grep -i memory
```

---

### Issue 9: Database Connection Errors

**Symptoms:**
```
ERROR: could not connect to database: connection refused
```

**Root Cause:** Database not accessible or incorrect credentials.

**Solution:**
```bash
# Verify database service
kubectl get svc postgres -n stellar

# Test connectivity
kubectl exec validator-0 -n stellar -- \
  nc -zv postgres 5432

# Check database credentials
kubectl get secret db-credentials -n stellar -o yaml

# Update database URL if needed
kubectl patch stellarvalidator validator \
  --type='merge' \
  -p '{"spec":{"config":{"databaseUrl":"postgresql://..."}}}'
```

---

### Issue 10: Slow Catchup Performance

**Symptoms:**
Catchup taking hours or days.

**Root Cause:** Insufficient resources or slow storage.

**Solution:**
```bash
# Increase catchup parallelism
kubectl patch stellarvalidator validator \
  --type='merge' \
  -p '{"spec":{"config":{"maxConcurrentSubprocesses":32}}}'

# Use faster storage class (SSD)
# Requires recreation of PVC
kubectl delete stellarvalidator validator -n stellar
# Edit YAML to use premium-ssd storage class
kubectl apply -f validator.yaml

# Increase CPU allocation
kubectl patch stellarvalidator validator \
  --type='merge' \
  -p '{"spec":{"resources":{"limits":{"cpu":"16"}}}}'
```

---

## Monitoring Issues

### Issue 11: Metrics Not Appearing in Prometheus

**Symptoms:**
No metrics visible in Prometheus for validator.

**Root Cause:** ServiceMonitor not created or Prometheus not configured.

**Solution:**
```bash
# Check ServiceMonitor
kubectl get servicemonitor -n stellar

# Verify Prometheus is scraping
kubectl port-forward -n monitoring svc/prometheus 9090:9090
# Visit http://localhost:9090/targets

# Enable monitoring if disabled
kubectl patch stellarvalidator validator \
  --type='merge' \
  -p '{"spec":{"monitoring":{"enabled":true,"serviceMonitor":true}}}'
```

---

### Issue 12: Grafana Dashboard Not Showing Data

**Symptoms:**
Dashboard imported but shows "No Data".

**Root Cause:** Incorrect datasource or label selectors.

**Solution:**
```bash
# Verify Prometheus datasource in Grafana
# Settings -> Data Sources -> Prometheus

# Check metrics are being collected
kubectl port-forward -n stellar validator-0 11626:11626
curl http://localhost:11626/metrics | grep stellar_core

# Update dashboard queries to match your label selectors
```

---

## Security Issues

### Issue 13: Network Policy Blocking Traffic

**Symptoms:**
Pods cannot communicate after applying NetworkPolicy.

**Root Cause:** Overly restrictive network policies.

**Solution:**
```bash
# Check network policies
kubectl get networkpolicies -n stellar

# Temporarily remove to test
kubectl delete networkpolicy validator-network-policy -n stellar

# Add necessary egress rules for DNS
# (see validator deployment guide for examples)
```

---

## Storage Issues

### Issue 14: Disk Full

**Symptoms:**
```
ERROR: disk full
```

**Root Cause:** Storage exhausted.

**Solution:**
See [Disk Scaling Guide](disk-scaling.md) for PVC expansion procedures.

---

## Additional Resources

- [Disk Scaling Troubleshooting](disk-scaling.md)
- [Sync Problems Guide](sync-problems.md)
- [Stellar Core Documentation](https://developers.stellar.org/docs/run-core-node)
- [Kubernetes Troubleshooting](https://kubernetes.io/docs/tasks/debug/)

!!! tip "Getting Help"
    If you encounter an issue not listed here:
    
    1. Check pod logs: `kubectl logs <pod-name> -n <namespace>`
    2. Check events: `kubectl get events -n <namespace> --sort-by='.lastTimestamp'`
    3. Describe resources: `kubectl describe <resource> <name> -n <namespace>`
    4. Search [GitHub Issues](https://github.com/OtowoOrg/Stellar-K8s/issues)
    5. Open a new issue with logs and configuration
