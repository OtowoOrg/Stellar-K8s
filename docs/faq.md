# Stellar-K8s Frequently Asked Questions (FAQ)

Welcome to the Stellar-K8s FAQ! This document addresses common questions and issues raised in support tickets, GitHub discussions, and the community.

**Table of Contents:**
- [Security](#security-questions) (mTLS, Certificates, Authentication)
- [Performance & Scaling](#performance--scaling-questions) (Storage, Disk Management, Resource Usage)
- [Troubleshooting](#troubleshooting-questions) (Health Checks, Peer Discovery, Networking)
- [General Operations](#general-operations-questions) (Deployment, Updates, Monitoring)

---

## Security Questions

### Q: What is mTLS and why do I need it?

**A:** mTLS (mutual TLS) ensures encrypted, authenticated communication between the operator and Stellar nodes. With mTLS enabled:
- The operator verifies node identity via certificates
- Nodes verify the operator's identity
- All traffic between operator and nodes is encrypted
- Protects against man-in-the-middle attacks

See [mTLS Setup and Certificate Rotation Guide](./mtls-guide.md) for detailed setup instructions.

### Q: How do I enable mTLS for the operator?

**A:** You can enable mTLS in two ways:

**Option 1: CLI**
```bash
stellar-operator run --namespace stellar-system --enable-mtls
```

**Option 2: Kubernetes Deployment**
```bash
kubectl -n stellar-system patch deployment stellar-operator \
  --type='json' \
  -p='[
    {"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--enable-mtls"}
  ]'
```

Alternatively, set the environment variable:
```bash
ENABLE_MTLS=true
```

See [mTLS Setup Guide - Enable mTLS](./mtls-guide.md#enable-mtls) for complete details.

### Q: How do I verify mTLS is properly configured?

**A:** Check for the presence of required secrets:

```bash
# Check operator CA and server certificate
kubectl -n stellar-system get secret stellar-operator-ca
kubectl -n stellar-system get secret stellar-operator-server-cert

# View secret data keys (should include tls.crt, tls.key, ca.crt)
kubectl -n stellar-system get secret stellar-operator-server-cert -o jsonpath='{.data}'

# Check node certificate secret (for node named 'validator-1')
kubectl -n stellar-system get secret validator-1-client-cert
```

All three secrets should exist with the correct data keys. See [mTLS Setup - Verify mTLS Provisioning](./mtls-guide.md#verify-mtls-provisioning).

### Q: What is the certificate rotation threshold and how do I customize it?

**A:** By default, the operator rotates server certificates 30 days before expiration. To customize this:

```bash
kubectl -n stellar-system set env deployment/stellar-operator CERT_ROTATION_THRESHOLD_DAYS=14
```

You can set any value in days. Lower values trigger more frequent rotations (more conservative), higher values rotate less frequently (takes more risk).

The operator checks expiry hourly and performs rotation automatically without restarting. See [mTLS Setup - Rotation](./mtls-guide.md#how-rotation-works).

### Q: How do I rotate node certificates manually?

**A:** For a node named `validator-1`:

1. Delete the node certificate secret:
```bash
kubectl -n stellar-system delete secret validator-1-client-cert
```

2. Trigger reconciliation by updating a node annotation:
```bash
kubectl -n stellar-system annotate stellarnode validator-1 \
  mtls.rotate-ts="$(date +%s)" --overwrite
```

The operator will recreate the certificate during the next reconciliation cycle. See [mTLS Setup - Manual Rotation](./mtls-guide.md#manual-rotation-runbooks).

### Q: Does Stellar-K8s support network isolation policies?

**A:** Yes! Stellar-K8s integrates with Kubernetes NetworkPolicies to enforce network segmentation. See [Network Isolation Guide](./network-isolation.md) for comprehensive policies.

Key ports to protect:
- **11625**: Stellar P2P (peer-to-peer consensus)
- **11626**: Stellar Admin HTTP (internal operators only)
- **8000**: Horizon REST API (external clients)
- **9100**: Prometheus metrics (monitoring only)

See [Troubleshooting - Networking](./troubleshooting/networking.md#quick-reference-stellar-port-map) for port reference.

### Q: How do I set up mutual TLS between validators?

**A:** mTLS in Stellar-K8s is configured at the operator level. Each validator automatically receives a client certificate that can be used for encrypted P2P communication. When you enable mTLS:

1. Operator creates CA and server cert in `stellar-operator-ca` secret
2. Operator creates per-node client cert in `<node-name>-client-cert` secret
3. Node mounts certificates at `/etc/stellar/tls/`

Multi-validator mutual authentication happens automatically. For external P2P communication with non-Kubernetes validators, configure your Stellar Core `stellar.conf` to use the client certificates. See [mTLS Setup - Certificate Model](./mtls-guide.md#certificate-and-secret-model).

---

## Performance & Scaling Questions

### Q: What is disk scaling and why is it important?

**A:** Disk scaling is the automatic expansion of storage volumes as the Stellar ledger grows. Without it, validators experience "Disk Full" outages which interrupt consensus. With disk scaling enabled:
- Operator continuously monitors disk usage
- Automatically expands volumes when usage exceeds a threshold (default: 80%)
- Prevents operator intervention and manual resizing
- Respects cloud provider limits  

See [Proactive Disk Scaling](./proactive-disk-scaling.md) for architecture and setup.

### Q: How do I configure disk scaling thresholds?

**A:** Configure disk scaling in your StellarNode manifest:

```yaml
spec:
  diskScaling:
    enabled: true                    # Enable/disable
    expansionThreshold: 80           # Trigger at 80% usage
    expansionIncrement: 50           # Increase by 50%
    minExpansionIntervalSecs: 3600   # Min 1 hour between expansions
    maxExpansions: 10                # Max 10 expansions total
```

**Example Conservative Settings (Lower Costs):**
```yaml
diskScaling:
  enabled: true
  expansionThreshold: 85             # Expand later (85% full)
  expansionIncrement: 30             # Smaller increases
  minExpansionIntervalSecs: 7200     # 2 hours between expansions
  maxExpansions: 15                  # More smaller expansions
```

**Example Aggressive Settings (Maximum Uptime):**
```yaml
diskScaling:
  enabled: true
  expansionThreshold: 70             # Expand early (70% full)
  expansionIncrement: 100            # Double the size
  minExpansionIntervalSecs: 1800     # 30 minutes between expansions
  maxExpansions: 8                   # Fewer larger expansions
```

See [Disk Scaling Quick Reference](./disk-scaling-quick-reference.md#configuration-cheat-sheet).

### Q: Why isn't disk scaling triggering for my node?

**A:** Check these conditions in order:

1. **Is disk scaling enabled?**
   ```bash
   kubectl get stellarnode <name> -o jsonpath='{.spec.diskScaling.enabled}'
   ```
   Should return `true`.

2. **Is usage above threshold?**
   ```bash
   kubectl get --raw /metrics | grep stellar_pvc_disk_usage_percent
   ```
   Check if usage exceeds your configured threshold.

3. **Has the minimum interval passed?**
   ```bash
   kubectl get pvc -n stellar-system -o \
     jsonpath='{.items[0].metadata.annotations.stellar\.org/last-disk-expansion}'
   ```

4. **Check StorageClass support:**
   ```bash
   kubectl get storageclass <name> -o jsonpath='{.allowVolumeExpansion}'
   ```
   Must return `true`.

5. **Review operator logs:**
   ```bash
   kubectl logs -n stellar-system -l app=stellar-operator | grep -i "disk\|expansion"
   ```

See [Disk Scaling Quick Reference - Troubleshooting](./disk-scaling-quick-reference.md#troubleshooting-checklist).

### Q: What are the cloud provider limits for volume expansion?

**A:** Each cloud provider has maximum volume sizes:

| Provider | Maximum Size | Volume Type | Note |
|----------|-------------|-------------|------|
| AWS | 16 TiB | gp3, io2 | Hard limit for most types |
| Google Cloud | 64 TiB | pd-ssd, pd-standard | Exceeds typical ledger growth |
| Azure | 32 TiB | Premium SSD | Exceeds typical ledger growth |
| Kubernetes | Varies | Depends on backing storage | Check provider documentation |

If you approach these limits, consider implementing archive pruning. See [History Archive Pruning](./archive-pruning.md).

### Q: How do I manually expand a PVC if automatic expansion fails?

**A:** Use `kubectl patch`:

```bash
kubectl patch pvc stellar-node-data -n stellar-system \
  -p '{"spec":{"resources":{"requests":{"storage":"200Gi"}}}}'
```

**Then verify:**
- Check `kubectl get pvc stellar-node-data -n stellar-system`
- Review pod logs for any issues: `kubectl logs -n stellar-system <pod-name>`
- Monitor disk usage to ensure expansion took effect

See [Disk Scaling Quick Reference - Manual Expansion](./disk-scaling-quick-reference.md#manual-expansion).

### Q: How long does ledger sync typically take?

**A:** Ledger sync time depends on:
- **Starting point**: How much history you have versus network
- **Network speed**: Ingest rate from peers
- **Hardware**: CPU and disk I/O performance
- **Testnet vs Mainnet**: Testnet has fewer ledgers (~5M), Mainnet has 50M+

**Rough estimates:**
- Testnet: 2-6 hours
- Mainnet from scratch: 24-72 hours
- Mainnet from recent checkpoint: 6-24 hours

Monitor sync progress:
```bash
kubectl get stellarnode <name> -o jsonpath='{.status.phase}'
# Shows: Pending, Creating, Syncing, Ready
```

See [Health Checks - Horizon Health Checks](./health-checks.md#horizon-health-checks) for monitoring details.

### Q: What resource requests should I set for Stellar nodes?

**A:** Recommended minimums:

| Node Type | CPU | Memory | Storage |
|-----------|-----|--------|---------|
| Validator | 2 | 4Gi | 200Gi+ |
| Horizon | 1 | 2Gi | 100Gi+ |
| Soroban RPC | 1 | 2Gi | 100Gi+ |

**For Production (Mainnet):**
| Node Type | CPU | Memory | Storage |
|-----------|-----|--------|---------|
| Validator | 4-8 | 8-16Gi | 500Gi+ |
| Horizon | 2-4 | 4-8Gi | 300Gi+ |
| Soroban RPC | 2-4 | 4-8Gi | 300Gi+ |

Example StellarNode configuration:
```yaml
spec:
  resources:
    requests:
      cpu: "4"
      memory: "8Gi"
    limits:
      cpu: "8"
      memory: "16Gi"
```

Storage automatically expands via disk scaling, so set an initial reasonable size. See [Resource Limits Documentation](./resource-limits.md).

---

## Troubleshooting Questions

### Q: How does peer discovery work in Stellar-K8s?

**A:** The operator automatically discovers other StellarNodes in the cluster and maintains a shared ConfigMap (`stellar-system/stellar-peers`) containing peer information. This enables validators to dynamically discover and connect without manual configuration.

**Discovery Components:**
1. **PeerDiscoveryManager** - Watches all StellarNode resources
2. **Shared ConfigMap** - Stores current peer list
3. **Config Reload Trigger** - Signals validators when peers change

**Peer ConfigMap Contents:**

`peers.json` - JSON array of discovered peers:
```json
[
  {
    "name": "validator-1",
    "namespace": "stellar-nodes",
    "nodeType": "Validator",
    "ip": "10.0.1.5",
    "port": 11625,
    "peerString": "10.0.1.5:11625"
  }
]
```

`peers.txt` - Simple list (one peer per line):
```
10.0.1.5:11625
10.0.1.6:11625
```

See [Dynamic Peer Discovery](./peer-discovery.md) for complete details.

### Q: Why isn't peer discovery finding my validators?

**A:** Check these conditions:

1. **Is the shared ConfigMap created?**
   ```bash
   kubectl get configmap -n stellar-system stellar-peers
   ```

2. **Are validators running and ready?**
   ```bash
   kubectl get stellarnode -A
   # Check status column - should show Ready
   ```

3. **Does the operator have RBAC permissions?**
   ```bash
   kubectl get clusterrole | grep stellar
   kubectl get clusterrolebinding | grep stellar
   ```

4. **Check operator logs:**
   ```bash
   kubectl logs -n stellar-system -l app=stellar-operator | grep -i "peer\|discovery"
   ```

5. **View the current peers:**
   ```bash
   kubectl get configmap -n stellar-system stellar-peers -o jsonpath='{.data.peers\.json}' | jq .
   ```

See [Peer Discovery](./peer-discovery.md) for troubleshooting.

### Q: How can I verify that validators are actually connecting to each other?

**A:** Check Stellar Core internal state:

```bash
# Port-forward to Core admin API
kubectl port-forward -n stellar-system validators/validator-1 11626:11626

# Query connected peers (from another terminal):
curl http://localhost:11626/peers

# Also check Core logs
kubectl logs -n stellar-system validator-1-0 -c stellar-core | grep "Connected to peer"
```

Or use the operator's built-in metrics:
```bash
kubectl get --raw /metrics | grep stellar_peers
```

### Q: What does it mean when a node is in "Syncing" phase?

**A:** The node is healthy (pod is running, HTTP endpoint responding) but hasn't yet caught up with the network's current ledger height. This is normal when:
- Node was just deployed
- Node was offline and is catching up
- Network is ahead of node's history

**Monitor sync progress:**
```bash
kubectl get stellarnode <name> -o jsonpath='{.status.phase},{.status.syncStatus}'
# Example output: Syncing,{"coreLatestLedger":50000000,"historyLatestLedger":49500000,"lag":500000}
```

**For Horizon nodes specifically:**
```bash
kubectl exec -n stellar-system <horizon-pod> -- \
  curl -s http://localhost:8000/health | jq .
```

A node transitions to `Ready` when the health endpoint confirms full sync. See [Health Checks](./health-checks.md).

### Q: What should I do if I see "Connection Refused" errors?

**A:** This means a connection was rejected at the target port. Troubleshoot in order:

1. **Is the pod running?**
   ```bash
   kubectl get pods -n <namespace> -l app.kubernetes.io/name=stellar-node
   kubectl describe pod <pod-name> -n <namespace>
   ```

2. **Is the Service pointing at correct pods?**
   ```bash
   kubectl get endpoints <service-name> -n <namespace>
   ```
   If empty, the selector doesn't match any pods.

3. **Is the port actually listening inside the container?**
   ```bash
   kubectl exec -n <namespace> <pod-name> -- ss -tlnp
   # or
   kubectl exec -n <namespace> <pod-name> -- netstat -tlnp 2>/dev/null
   ```

4. **Test from another pod in the cluster:**
   ```bash
   kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -- \
     nc -zv <service-name>.<namespace>.svc.cluster.local 11625
   ```

See [Networking Troubleshooting Guide](./troubleshooting/networking.md#1-diagnosing-connection-refused).

### Q: What does "No Route to Host" error mean?

**A:** The packet never reached the destination — usually due to NetworkPolicies, firewalls, or CNI misconfiguration.

**Check NetworkPolicies:**
```bash
kubectl get networkpolicies -n <namespace>
kubectl describe networkpolicy <name> -n <namespace>
```

**If you have a default-deny policy, allow P2P traffic:**
```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-stellar-p2p
  namespace: stellar-system
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/component: stellar-validator
  policyTypes:
    - Ingress
    - Egress
  ingress:
    - ports:
        - port: 11625
          protocol: TCP
  egress:
    - ports:
        - port: 11625
          protocol: TCP
```

See [Networking Troubleshooting Guide](./troubleshooting/networking.md#2-diagnosing-no-route-to-host).

### Q: How do I enable mTLS if I forgot to configure it initially?

**A:** You can enable mTLS on an existing operator without data loss:

1. **Patch the deployment:**
   ```bash
   kubectl -n stellar-system patch deployment stellar-operator \
     --type='json' \
     -p='[
       {"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--enable-mtls"}
     ]'
   ```

2. **Wait for new operator pod to start:**
   ```bash
   kubectl rollout status deployment/stellar-operator -n stellar-system
   ```

3. **Verify certificates were created:**
   ```bash
   kubectl get secrets -n stellar-system | grep stellar-operator
   ```

4. **Node certificates are auto-provisioned** on next reconciliation:
   ```bash
   kubectl get secrets -n stellar-system | grep client-cert
   ```

There's no downtime during this process. See [mTLS Setup - Enable mTLS](./mtls-guide.md#enable-mtls).

---

## General Operations Questions

### Q: How do I upgrade the Stellar-K8s operator?

**A:** Using Helm (recommended):

```bash
# Update Helm repository
helm repo update stellar-k8s

# Upgrade the operator
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --values my-values.yaml
```

**Using kubectl directly:**

1. Get the current image version
2. Update the deployment
3. Monitor rollout

See [Deployment Guide](./deploy-olm.md) and [hitless-upgrade.md](./hitless-upgrade.md) for zero-downtime upgrades.

### Q: How do I monitor the operator's health?

**A:** Check multiple dimensions:

1. **Operator pod status:**
   ```bash
   kubectl get pods -n stellar-system -l app=stellar-operator
   kubectl logs -n stellar-system -l app=stellar-operator --tail=100
   ```

2. **Reconciliation loops:**
   ```bash
   kubectl logs -n stellar-system -l app=stellar-operator | grep -i "reconcil\|error"
   ```

3. **Prometheus metrics (if enabled):**
   ```bash
   kubectl get --raw /metrics | grep stellar_reconcile
   ```

4. **Node status:**
   ```bash
   kubectl get stellarnode -A
   ```

### Q: Can I run multiple instances of the Stellar-K8s operator?

**A:** No. The operator uses leader election to ensure only one instance actively reconciles at a time. You can deploy multiple replicas for high availability, but only one will be active. The standby replicas automatically take over if the active instance fails.

**Deploy with HA:**
```yaml
spec:
  replicas: 3  # Or more
```

The operator handles leader election automatically using Kubernetes leases.

### Q: What Kubernetes versions are supported?

**A:** Stellar-K8s requires Kubernetes 1.28 or later. We test and support:
- Kubernetes 1.28 (LTS)
- Kubernetes 1.29
- Kubernetes 1.30 (Current)

Check your cluster version:
```bash
kubectl version
```

### Q: How do I back up my Stellar node data?

**A:** Stellar-K8s uses PersistentVolumeClaims for storage. Back up strategies:

1. **Cloud-native snapshots (recommended):**
   ```bash
   # AWS (EBS Snapshots)
   kubectl get volumesnapshot -n stellar-system
   ```

2. **Manual backup using `rsync`:**
   ```bash
   kubectl exec -n stellar-system validator-1-0 -- \
     rsync -av /data/ /mnt/backup/
   ```

3. **Use external backup tools** (Velero, etc.)

See [Backup Verification](./backup-verification.md) and [Volume Snapshots](./volume-snapshots.md).

### Q: How is archive pruning different from disk scaling?

**A:** They serve different purposes:

| Feature | Disk Scaling | Archive Pruning |
|---------|--------------|-----------------|
| **Purpose** | Expand storage | Delete old data |
| **When It Runs** | When usage exceeds threshold | On schedule (cron) |
| **What It Does** | Increases PVC size | Removes old history checkpoints |
| **Cost Impact** | Increases costs (more storage) | Decreases costs (less storage) |
| **Use Case** | Short-term growth management | Long-term storage optimization |

**Best practice:** Use disk scaling for immediate capacity needs and pruning for long-term cost optimization on Mainnet.

See [Archive Pruning](./archive-pruning.md) and [Proactive Disk Scaling](./proactive-disk-scaling.md).

---

## Related Documentation

- [Getting Started Guide](./getting-started.md)
- [API Reference](./api-reference.md)
- [mTLS Setup and Certificate Rotation](./mtls-guide.md)
- [Network Isolation Guide](./network-isolation.md)
- [Health Checks Documentation](./health-checks.md)
- [Dynamic Peer Discovery](./peer-discovery.md)
- [Proactive Disk Scaling](./proactive-disk-scaling.md)
- [Archive Pruning Integration Guide](./PRUNING_INTEGRATION_GUIDE.md)
- [Troubleshooting Guide](./troubleshooting/networking.md)
- [Glossary](./glossary.md)

## Support Resources

- **GitHub Issues**: [Stellar-K8s Issues](https://github.com/OtowoOrg/Stellar-K8s/issues)
- **GitHub Discussions**: [Stellar-K8s Discussions](https://github.com/OtowoOrg/Stellar-K8s/discussions)
- **Stellar Developer Docs**: [developers.stellar.org](https://developers.stellar.org/)
- **Kubernetes Documentation**: [kubernetes.io](https://kubernetes.io/)

---

**Last Updated**: April 2026

Have a question not covered here? Please open an issue on [GitHub](https://github.com/OtowoOrg/Stellar-K8s/issues) or start a discussion!
