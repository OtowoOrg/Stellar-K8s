# Disaster Recovery: Rebuilding Quorum After Complete Datacenter Loss

This runbook provides the procedure for restoring a Tier-1 Stellar validator cluster after a complete datacenter or region loss. This ensures institutional node operators can survive catastrophic infrastructure loss within strict Service Level Agreements (SLAs).

> **[!WARNING]**
> **CRITICAL CONSTRAINT: NEVER RUN DUPLICATE KEYS SIMULTANEOUSLY.**
> Running the same validator seed keys simultaneously in separate datacenters (e.g., if the old datacenter unexpectedly comes back online while the new one is running) will lead to immediate protocol-level blacklisting and network fracture. Ensure the old environment is fully decommissioned or definitively fenced off before importing keys to the new environment.

## 1. Restoring the Kubernetes Environment

1. **Provision New Infrastructure**: Spin up a new Kubernetes cluster in the designated DR region.
2. **Restore etcd (If Applicable)**: If your cluster requires stateful etcd restoration, follow your cloud provider's guide to restore etcd from the latest snapshot. (For stateless stellar-operator deployments, this step may be skipped, relying instead on fresh CRD application).
3. **Deploy the Stellar Operator**: 
   Apply the stellar-operator to the new cluster:
   ```bash
   kubectl apply -f deploy/crds/
   kubectl apply -f deploy/operator.yaml
   ```

## 2. Securely Importing Validator Seed Keys

To avoid exposing your highly sensitive validator seed keys, they should be injected directly from a secure storage mechanism (like HashiCorp Vault or an HSM).

1. **Authenticate to Vault**: Ensure your environment has the necessary tokens or IAM roles to read from the Vault secrets path.
2. **Execute Recovery Script**: Run the provided Vault recovery script to fetch the keys and generate the Kubernetes Secret locally.
   ```bash
   ./examples/dr/vault-recovery.sh
   ```
3. **Apply the Generated Secret**:
   ```bash
   kubectl apply -f stellar-validator-secret.yaml
   ```
   *(Note: Ensure this file is deleted immediately after application to avoid leaving secrets on disk).*
4. **Deploy the Validator CRD**: Apply your Stellar Core CRD, referencing the created secret:
   ```yaml
   apiVersion: stellar.k8s.io/v1alpha1
   kind: StellarCore
   metadata:
     name: tier1-validator
   spec:
     network: public
     validatorSecretName: stellar-validator-keys
   ```

## 3. Network Sync and Peer Connectivity Checklist

Before officially announcing the node's return to the public quorum, strictly verify the following:

- [ ] **Pod Status**: Verify that the `stellar-core` pods are running and stable without crash loops.
  ```bash
  kubectl get pods -l app=stellar-core
  ```
- [ ] **State Sync**: Verify the node is syncing with the network and the ledger number is advancing. Check the `stellar-core` logs or the `/info` endpoint.
  ```bash
  curl -s http://<stellar-core-ip>:11626/info | jq '.info.state'
  # Expected: "Synced!"
  ```
- [ ] **Peer Connectivity**: Confirm the node is connected to a sufficient number of trusted peers.
  ```bash
  curl -s http://<stellar-core-ip>:11626/peers | jq '.authenticated_peers.count'
  # Expected: Greater than 0, ideally matching your quorum set configuration.
  ```
- [ ] **Quorum Health**: Check that the node is participating in consensus.
  ```bash
  curl -s http://<stellar-core-ip>:11626/quorum | jq '.node'
  # Expected: "tracking" or "agreeing"
  ```
- [ ] **Old Environment Fencing**: Double-check that all networking/routing to the old datacenter is severed, preventing any possibility of a split‑brain scenario.

Once all checklist items are verified, you may safely announce the node's successful recovery and resume normal operations.
