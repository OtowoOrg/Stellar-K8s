# Production-oriented HashiCorp Vault + Stellar validators

This tutorial matches the native **`vaultRef`** seed source on `StellarNode` (see [`src/crd/seed_secret.rs`](../src/crd/seed_secret.rs)) and the Vault Agent Injector sidecar pattern.

## 1. Install Vault with injector

```bash
helm repo add hashicorp https://helm.releases.hashicorp.com
helm repo update
helm install vault hashicorp/vault --set "injector.enabled=true" --namespace vault --create-namespace
```

Configure Kubernetes auth, a policy, and a role bound to the validator `ServiceAccount` (namespace `stellar` in this example).

## 2. Store the validator seed in KV v2

```bash
kubectl exec -n vault vault-0 -- vault kv put secret/stellar/validator seed="SXXXX..."
```

## 3. StellarNode with `vaultRef`

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: prod-validator
  namespace: stellar
spec:
  nodeType: Validator
  network: Public
  version: "v21.0.0"
  storage:
    storageClass: fast-ssd
    size: 200Gi
    retentionPolicy: Retain
  validatorConfig:
    seedSecretSource:
      vaultRef:
        role: stellar-validator
        secretPath: secret/data/stellar/validator
        secretKey: seed
        secretFileName: stellar-seed
        restartOnSecretRotation: true
```

The operator injects standard `vault.hashicorp.com/*` pod annotations; the Agent renders `/vault/secrets/stellar-seed` and the main container receives `STELLAR_SEED_FILE`.

## 4. Secret rotation and restarts

With `restartOnSecretRotation: true`, the operator compares the injector’s `vault.hashicorp.com/secret-version-stellar-seed` annotation on pods with `status.vaultObservedSecretVersion` and triggers a rolling annotation bump on the StatefulSet when Vault serves a new version.

## 5. manifests/

Example supporting resources (policy snippets, ServiceAccount binding) live under [`examples/vault/`](../examples/vault/) in this repository.
