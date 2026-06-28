# Sync Problems Troubleshooting

This guide describes how to identify, troubleshoot, and resolve issues related to Stellar Core ledger synchronization and catchup lag.

---

## 1. Checking Sync Status
To verify the sync state of a deployed Stellar node:
1. Use the `kubectl-stellar` plugin:
   ```bash
   kubectl stellar status
   ```
2. Query Stellar Core directly via the admin HTTP API:
   ```bash
   kubectl exec my-validator-0 -n stellar -c stellar-node -- \
     curl -s http://localhost:11626/info | jq '.info.state'
   ```
   **Expected States:**
   - `"Synced!"`: Node is fully synchronized with the network.
   - `"Catching up"`: Node is downloading history checkpoints.
   - `"Joining SCP"`: Node is waiting to hear from quorum before joining consensus.

---

## 2. Common Sync Issues

### 2.1 Node Stuck in "Joining SCP"
**Symptoms:**
State remains `"Joining SCP"` indefinitely. The node does not download new ledgers.

**Root Causes:**
1. **Network Passphrase mismatch**: The validator is configured with a different network passphrase than its peers.
2. **Quorum set unreachable**: The validator cannot reach the peers specified in its quorum set.
3. **Insufficient peers in quorum set**: Not enough validators in the quorum are active to reach a threshold consensus.

**Diagnosis:**
1. Check the network passphrase config:
   ```bash
   kubectl get configmap my-validator-config -n stellar -o yaml | grep NETWORK_PASSPHRASE
   ```
2. Check connectivity to other validators in the quorum:
   ```bash
   kubectl exec my-validator-0 -n stellar -c stellar-node -- \
     nc -zv peer-validator-service-ip 11625
   ```

### 2.2 Slow Catchup Performance
**Symptoms:**
Node is in `"Catching up"` state but progress is extremely slow (e.g., catching up 1 ledger per second).

**Solutions:**
1. **Configure catchup concurrency**:
   Stellar Core uses subprocesses to download and verify checkpoints. Increase concurrency in `StellarNode` spec:
   ```yaml
   spec:
     validatorConfig:
       maxConcurrentSubprocesses: 32
   ```
2. **Enable Captive Core mode**:
   Captive Core runs Stellar Core with an in-memory database during catchup, avoiding database write bottlenecks.
3. **Use fast persistent storage**:
   Slow storage limits sqlite/postgresql write throughput. Switch the node to NVMe storage by setting `mode: Local` under storage spec.

---

## 3. History Archive Failures
Stellar Core requires access to history archives (e.g., Amazon S3 or Google Cloud Storage buckets) to perform catchup.
- **Verification**: Check if the archives are readable from the pod:
  ```bash
  kubectl exec my-validator-0 -n stellar -c stellar-node -- \
    curl -I https://history.stellar.org/prd/core-live/core_live_001/
  ```
- **Error in logs**:
  ```
  Failed to download history archive file...
  ```
  Ensure firewall rules permit outbound access on port `443`.
