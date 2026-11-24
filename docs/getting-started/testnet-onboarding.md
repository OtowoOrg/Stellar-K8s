# Testnet Onboarding Guide

Welcome to Stellar-K8s testnet! This guide walks you through joining the testnet ecosystem, from prerequisites to full node operation.

## Quick Navigation

- **New to Stellar?** Start with [Glossary](../glossary.md)
- **Ready to deploy?** Jump to [Deploy a Testnet Validator](../tutorials/deploy-testnet-validator.md)
- **Networking issues?** See [Testnet Peer Egress Port Requirements](../networking/testnet-egress-ports.md)
- **Questions?** Check [FAQ](../faq.md) or [report an issue](../issue-reporting.md)

---

## What is Testnet?

The **Stellar Testnet** is a separate blockchain network maintained by the Stellar Development Foundation (SDF). It is identical in protocol to mainnet but runs on different hardware and has its own ledger, accounts, and validator quorum.

| Property | Testnet | Mainnet |
|----------|---------|---------|
| Purpose | Development, testing, learning | Production blockchain |
| Network passphrase | `Test SDF Network ; September 2015` | `Public Global Stellar Network ; September 2015` |
| Peer port(s) | `3510`, `11625` TCP | `11625` TCP |
| Validator count | ~6 SDF-operated cores | 50+ community validators |
| Reset policy | Resets **monthly** — plan accordingly | Never resets |
| Account lifetimes | 30 days of inactivity → purged | Permanent (unless deleted) |

!!! warning "Testnet ≠ Sandbox"
    Testnet is a real blockchain with real consensus. Your testnet validators participate in the actual network; they don't run in isolation. Don't treat testnet as a sandbox for untested code.

---

## Onboarding Checklist

### Phase 1: Planning (Before you deploy)

- [ ] **Read the basics**
  - Review the [Glossary](../glossary.md) (validator, quorum set, consensus, SCP)
  - Understand [Architecture Overview](../architecture.md)

- [ ] **Understand networking**
  - Read [Testnet Peer Egress Port Requirements](../networking/testnet-egress-ports.md) **in full** — this is the #1 blocker for new validators
  - Identify your egress policy: cloud SG, NAT gateway, corporate firewall, or CNI

- [ ] **Verify prerequisites**
  - Kubernetes cluster (1.28+) with 4+ CPU and 8+ GB RAM available
  - kubectl configured and authenticated
  - Stellar-K8s operator installed ([installation guide](installation.md))
  - Outbound TCP `3510` and `11625` open to SDF testnet cores

### Phase 2: Setup (Deploy your validator)

- [ ] **Generate keypair**
  - Use `stellar-core gen-seed` (Docker) or `stellar-cli keys generate`
  - Store the secret seed securely — you'll need it for the Kubernetes secret

- [ ] **Create Kubernetes namespace**
  - `kubectl create namespace stellar-testnet` (or your preferred name)
  - Apply [network policies](../networking/index.md) if you run a default-deny cluster

- [ ] **Create secret**
  - Store your validator seed in a Kubernetes secret
  - Example: `kubectl create secret generic validator-seed --from-literal=seed='SB...' -n stellar-testnet`

- [ ] **Deploy validator**
  - Follow the [step-by-step tutorial](../tutorials/deploy-testnet-validator.md)
  - Use the example manifest provided — it includes all testnet-specific settings

- [ ] **Verify pod health**
  - Pod should reach `Running` state within 1–2 minutes
  - Check logs: `kubectl logs -n stellar-testnet <pod> -f`
  - Pod should report HTTP admin endpoint ready on port `11626`

### Phase 3: Validation (Confirm it's working)

- [ ] **Check peer connectivity**
  - Command: `kubectl exec -n stellar-testnet <pod> -- curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'`
  - Expected: ≥ 1 peer within 2–5 minutes of pod start
  - If `0`, review [Testnet Peer Egress Port Requirements](../networking/testnet-egress-ports.md#8-symptom-checklist-zero-peers--joining-scp)

- [ ] **Check sync status**
  - Command: `kubectl exec -n stellar-testnet <pod> -- curl -s http://localhost:11626/info | jq '.state'`
  - States you'll see:
    - `Joining SCP` — normal for first few minutes; node is discovering quorum
    - `Synced!` — healthy; you're caught up with the network
    - `Catching up` — normal; node is replaying recent ledgers

- [ ] **Confirm ledger progress**
  - Command: `kubectl exec -n stellar-testnet <pod> -- curl -s http://localhost:11626/info | jq '.ledger'`
  - Ledger sequence should increase every ~5 seconds on testnet
  - If ledger is frozen, you're stuck; cross-check [readiness states](../operations/readiness-probe-states.md)

- [ ] **Check metrics (if monitoring enabled)**
  - Pod should export Prometheus metrics on port `9090` (if `monitoring.enabled: true`)
  - Example: `kubectl port-forward -n stellar-testnet <pod> 9090:9090 &`
  - Then visit `http://localhost:9090/metrics` and grep for `stellar_ledger_`

### Phase 4: Operations (Ongoing management)

- [ ] **Monitor readiness**
  - The operator's readiness probe reflects [documented states](../operations/readiness-probe-states.md)
  - Unhealthy states (e.g., `Joining SCP`, network isolation) remove the pod from Service endpoints
  - Watch `kubectl get pods -n stellar-testnet -w`

- [ ] **Plan for testnet resets**
  - Testnet resets monthly — all ledger data is lost
  - Set up history archive management to preserve pre-reset state ([archive-pruning.md](../archive-pruning.md))
  - Plan validator account recreation post-reset

- [ ] **Join monitoring / community**
  - Subscribe to [Stellar blog](https://stellar.org/blog) for testnet maintenance announcements
  - Join the [Stellar Developers Discord](https://discord.gg/stellardev) to discuss issues and upgrades

---

## Deployment Patterns

### Single Testnet Validator (Evaluation)

Use this if you're testing the operator, learning Kubernetes, or evaluating Stellar-K8s.

**Characteristics:**
- 1 pod, no HA
- Basic storage (EBS/GCP Standard)
- Minimal CPU/memory (2 vCPU, 4 GB RAM requested)
- Good for development and sandbox testing

**Deploy:** Follow the [step-by-step tutorial](../tutorials/deploy-testnet-validator.md) — it uses this pattern by default.

### High-Availability Validator Cluster (Production Testnet)

Use this if you're running a public testnet node for a project or testing HA failover.

**Characteristics:**
- Multiple stateful replicas (2–3 recommended)
- Shared persistent storage (EBS snapshot, RBD, GCS)
- Pod disruption budget (PDB) for safe node maintenance
- Readiness and liveness probes
- Monitoring + alerting

**Deploy:** See [Configure HA Setup](../tutorials/configure-ha-setup.md) — testnet HA is identical to mainnet HA except for network passphrase and peer ports.

### GitOps Validator (ArgoCD/Flux)

Use this if you're managing infrastructure as code and want declarative, versioned validator configuration.

**Characteristics:**
- Validator manifest in Git
- Automatic reconciliation via ArgoCD or Flux
- Easy rollback and history
- Integration with release pipelines

**Deploy:** See the [ArgoCD integration guide](../gitops/argocd.mdx) — includes testnet example manifests.

---

## Common Issues & Quick Fixes

### Pod is stuck at `Joining SCP` with 0 peers

**Diagnosis:**
```bash
kubectl exec -n stellar-testnet <pod> -- curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'
# Expected: >= 1
# If 0: egress is likely blocked
```

**Fix:**
1. Review [Testnet Peer Egress Port Requirements](../networking/testnet-egress-ports.md#8-symptom-checklist-zero-peers--joining-scp)
2. Verify outbound TCP `3510` and `11625` are open to testnet cores
3. Check cloud security groups, NAT gateways, and in-cluster network policies
4. Test from a debug pod in the same namespace: `kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -n stellar-testnet -- nc -zv -w 5 <sdf-testnet-host> 3510`

### Pod is not starting / CrashLoopBackOff

**Diagnosis:**
```bash
kubectl describe pod -n stellar-testnet <pod>
kubectl logs -n stellar-testnet <pod> --previous
```

**Common causes:**
- Missing validator seed secret: verify `kubectl get secret -n stellar-testnet <secret-name>`
- Insufficient compute resources: check node `kubectl top nodes` and pod requests in manifest
- Volume mount errors: verify PVC exists and is `Bound` with `kubectl get pvc -n stellar-testnet`

### Validator will not sync / stuck in `Catching up`

**Diagnosis:**
```bash
# Check ledger sequence — should advance every ~5s
kubectl exec -n stellar-testnet <pod> -- curl -s http://localhost:11626/info | jq '.ledger'

# Watch logs for sync errors
kubectl logs -n stellar-testnet <pod> -f | grep -i catch
```

**Common causes:**
- Disk full or nearly full (check PVC capacity)
- Testnet is offline or reset is imminent (check [Stellar status page](https://status.stellar.org))
- History archive is unavailable (check `http://localhost:11626/info` → `historyIsComplete`)

---

## What's Next?

- **Deploy Horizon:** [Deployment Guide](../deployment-guides/horizon.md) — enables REST API access to ledger data
- **Integrate monitoring:** [Monitoring Setup Guide](../MONITORING_SETUP_GUIDE.md) — Prometheus + Grafana dashboards
- **Test failover:** [Chaos Engineering Guide](../chaos-drills.md) — validate HA with controlled failure injection
- **Go mainnet:** [Mainnet deployment guide](../deployment-guides/validator.md) — everything you've learned applies with just a network passphrase change

---

## Support & Feedback

- **Found a bug?** [Report it on GitHub](../issue-reporting.md)
- **Have a question?** Check the [FAQ](../faq.md) or ask in the [Stellar Discord](https://discord.gg/stellardev)
- **Want to contribute?** See the [Contributing Guide](../contributing/index.md)

