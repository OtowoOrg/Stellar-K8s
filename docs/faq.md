# Stellar-K8s Frequently Asked Questions (FAQ)

Welcome to the Stellar-K8s FAQ! This document addresses common questions and issues raised in support tickets, GitHub discussions, and the community.

**Table of Contents:**
- [Security](#security-questions) (mTLS, Certificates, Authentication)
- [Performance & Scaling](#performance-scaling-questions) (Storage, Disk Management, Resource Usage)
- [Troubleshooting](#troubleshooting-questions) (Health Checks, Peer Discovery, Networking)
- [General Operations](#general-operations-questions) (Deployment, Updates, Monitoring)
- [Deployment FAQ](#deployment-faq) (First Validator, Testnet Onboarding)
- [Documentation & Build](#documentation-build-questions) (Deployment Guides, Observability, Make Targets)

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

**A:** Yes! Stellar-K8s integrates with Kubernetes NetworkPolicies to enforce network segmentation. See [Network Isolation Guide](./network-isolation.md) for the architecture and [Network Policy Templates](./network-policy-templates.md) for ready-to-apply patterns.

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

See [Disk Scaling Quick Reference](./proactive-disk-scaling.md#configuration).

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

See [Disk Scaling Quick Reference - Troubleshooting](./proactive-disk-scaling.md#troubleshooting).

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

See [Disk Scaling Quick Reference - Manual Expansion](./proactive-disk-scaling.md#manual-expansion).

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

### Q: What does "Joining SCP" mean, and when is it a problem?

**A:** `Joining SCP` is a transitional Stellar Core state meaning the node is up and has connected to peers, and is now exchanging SCP messages to join consensus for the first time — the handshake layer works, consensus participation hasn't started yet. It is the expected state for a freshly deployed validator and normally lasts a **few minutes**; the node then advances to `Synced!` (or first to `Catching up` on a fresh volume). Note that it is distinct from `Catching up` (replaying ledger history — can legitimately take hours) and from `Syncing` (see [the previous question](#q-what-does-it-mean-when-a-node-is-in-syncing-phase)).

**What to check:** query the state field from the Core admin API:

```bash
# .info.state is the field to inspect (returns "Joining SCP", "Synced!", ...)
kubectl exec -n <namespace> <validator-pod> -- curl -s http://localhost:11626/info | jq -r '.info.state'

# Connected peer count — the key discriminator, see below
kubectl exec -n <namespace> <validator-pod> -- curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'
```

**When it's normal:**

- Fresh deployment; peers connected (`/peers` count > 0); state advances to `Synced!` within minutes.
- First boot after volume recreation or a catch-up restart.

**When it indicates a connectivity problem:** the pod runs, `/info` keeps returning `Joining SCP`, and the peer count stays at **0**. The operator's own alerting encodes this threshold — `StellarCoreNoPeers` fires critical at 5 minutes of zero peers ([alert rules](https://github.com/OtowoOrg/Stellar-K8s/blob/main/monitoring/stellar-core-metrics.yaml)). With zero peers the node can never see a quorum, so it will sit in `Joining SCP` indefinitely. The usual cause is blocked **outbound** access to peer ports (commonly missed in firewalls, NAT gateways, and cloud security groups). Work through the [Networking Troubleshooting Guide](./troubleshooting/networking.md) — start with §6.1 "Required outbound connections from validators" and the P2P firewalling checks — then the [Network Configuration Guide](./networking/index.md) for egress hardening, and the [Peer Discovery FAQ](#q-how-does-peer-discovery-work-in-stellar-k8s) if the peer list itself is empty.

While stuck, the [readiness probe](./operations/readiness-probe-states.md) keeps the pod **Not Ready** (`Joining SCP` is not a ready state), so the node is also removed from Service endpoints.

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

## Deployment FAQ

### Q: How do I deploy my first validator on the public testnet? (Testnet onboarding start to finish)

**A:** Testnet is the right place to start: it uses the same operator workflow as Mainnet, but the ledger is small (~5M ledgers vs 50M+), sync is fast (2–6 hours on modest hardware), and mistakes cost nothing. Here is the complete path from zero to a synced testnet validator:

**1. Prerequisites and operator installation.** Work through the [Getting Started](./getting-started/index.md) track: [Prerequisites](./getting-started/prerequisites.md) → [Installation](./getting-started/installation.md) → [Quick Start](./getting-started/quick-start.md). You need a Kubernetes 1.28+ cluster, kubectl, and Helm 3.x.

**2. Create the seed secret.** Generate a keypair and store the secret seed in Kubernetes (never commit seeds or put them in the manifest):

```bash
docker run --rm stellar/stellar-core:latest stellar-core gen-seed
kubectl create secret generic validator-seed-testnet \
  --from-literal=seed='SBXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX' \
  -n stellar
```

For KMS/Vault-backed seed handling and rotation, see [Secrets Management](./secret-management-kms.md).

**3. Apply the example manifest.** Use [`examples/validator-testnet.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/validator-testnet.yaml) as-is for your first boot — it is the canonical source for Testnet defaults (`network: testnet` selects the SDF Test Network passphrase and default history archives, with balanced 500m/1Gi requests and 100Gi storage):

```bash
kubectl apply -f examples/validator-testnet.yaml
kubectl get stellarnodes -n stellar
```

**4. First boot.** The pod starts, opens outbound peer connections, and progresses through the readiness states (`Booting` → `Joining SCP` → `Synced!`). Watch it:

```bash
kubectl logs -n stellar -f $(kubectl get pods -n stellar -o name | head -1)
kubectl exec -n stellar <validator-pod> -- curl -s http://localhost:11626/info
```

`Joining SCP` for a while is normal; what you want to see next is `Synced!`. The state machine behind the readiness probe is documented in [Readiness Probe States](./operations/readiness-probe-states.md).

**5. Verify sync.** A synced validator reports `Synced!` from the `/info` endpoint and is added back to the Service endpoints by the operator.

**Where to go next:**

- Step-by-step walkthrough with screenshots of each check: [Deploy a Testnet Validator](./tutorials/deploy-testnet-validator.md)
- Full deployment guide (quorum sets, archives, production options): [Validator Deployment Guide](./deployment-guides/validator.md)
- Two facts to keep straight from day one: network passphrases and seeds. The **passphrase is derived from your manifest's `network:` field — don't hardcode it** (`testnet` → `Test SDF Network ; September 2015`, `mainnet` → `Public Global Stellar Network ; September 2015`); see [Network Isolation](./network-isolation.md) for why cross-network mixing is dangerous. **Seeds are always referenced via `validatorConfig.seedSecretRef`** pointing at a Kubernetes Secret, as in step 2 above — the [API reference for `seedSecretRef`](./api-reference.md) covers the field's exact behavior.
- Testnet firewall notes (outbound peer ports) live in the [Network Configuration Guide](./networking/index.md)

---

## Documentation & Build Questions

### Q: Where are the deployment guides?

**A:** Start at the [Deployment Guides index](./deployment-guides/index.md). It links the Validator, Horizon, and Soroban RPC guides, plus the deployment patterns and tutorials.

### Q: Where do I find observability documentation?

**A:** Start at the [Observability overview](./observability/index.md). It covers metrics, dashboards, logging, tracing, and the telemetry contract.

### Q: How do I build the operator locally?

**A:** Use the Makefile targets:

```bash
make build-debug   # unoptimised binary, fastest iteration
make build         # optimised release binary (--locked)
make build-check   # type-check all targets without producing binaries
make build-all     # release binary plus the documentation site
```

Run `make help` to list every target.

### Q: How do I build and preview the documentation site?

**A:** Run `make docs-build` to build the site into `site/`, or `make docs-serve` to serve it at `http://127.0.0.1:8000`.

### Q: Which build target should I use for a fast compile check?

**A:** Use `make build-check` — it runs `cargo check --locked --all-targets` and type-checks every target without producing binaries, so it is much faster than a full `make build`. For an even quicker pre-commit loop, `make quick` combines the format check with a compile check. See the [Build Targets reference](./development/build-targets.md) for the full table of build-related targets.

### Q: How do I run the documentation quality gates locally before opening a PR?

**A:** Three targets mirror the documentation CI jobs:

```bash
make nav-coverage   # every mkdocs.yml nav entry resolves to a real file (#1711)
make docs-ci-build  # strict mkdocs build — fails on broken links or nav warnings (#1710)
make link-integrity # internal markdown links across docs/ are valid (#1712)
```

See [Navigation Coverage](./development/docs-nav-coverage.md), [Documentation Build](./development/docs-build.md), and [Link Integrity](./development/docs-link-integrity.md).

### Q: Where can I find the metrics the operator and nodes expose?

**A:** The [Metric Reference](./observability/metric-reference.md) lists every metric with its meaning and labels, the [Metric Naming Conventions](./observability/metric-naming-conventions.md) cover naming rules for new metrics, and the [Observability overview](./observability/index.md) is the entry point for metrics, logs, tracing, and dashboards. The operator REST metrics endpoint is documented under [Metrics API](./api/metrics.md).

### Q: Which deployment guide should I follow for my node type?

**A:** Start at the [Deployment Guides index](./deployment-guides/index.md):

| You want to run | Guide |
|-----------------|-------|
| Stellar Core validator | [Validator Node](./deployment-guides/validator.md) |
| Horizon REST API | [Horizon API](./deployment-guides/horizon.md) |
| Soroban RPC endpoint | [Soroban RPC](./deployment-guides/soroban-rpc.md) |
| Operator on OpenShift / OLM | [OLM / OpenShift](./deploy-olm.md) |

For production topologies (HA validators, Horizon/RPC farms, multi-region DR, GitOps rollouts) see the [Deployment Patterns](./deployment-patterns/index.md) reference architectures.

## Related Documentation

- [Getting Started Guide](./getting-started/quick-start.md)
- [API Reference](./api-reference.md)
- [mTLS Setup and Certificate Rotation](./mtls-guide.md)
- [Network Isolation Guide](./network-isolation.md)
- [Pod-to-Pod Network Segmentation](./network-pod-to-pod.md)
- [Health Checks Documentation](./health-checks.md)
- [Dynamic Peer Discovery](./peer-discovery.md)
- [Proactive Disk Scaling](./proactive-disk-scaling.md)
- [Archive Pruning Guide](./archive-pruning.md)
- [Troubleshooting Guide](./troubleshooting/networking.md)
- [Glossary](./glossary.md)

## Support Resources

- **GitHub Issues**: [Stellar-K8s Issues](https://github.com/OtowoOrg/Stellar-K8s/issues)
- **GitHub Discussions**: [Stellar-K8s Discussions](https://github.com/OtowoOrg/Stellar-K8s/discussions)
- **Stellar Developer Docs**: [developers.stellar.org](https://developers.stellar.org/)
- **Kubernetes Documentation**: [kubernetes.io](https://kubernetes.io/)

---

**Last Updated**: October 2026

Have a question not covered here? Please open an issue on [GitHub](https://github.com/OtowoOrg/Stellar-K8s/issues) or start a discussion!
