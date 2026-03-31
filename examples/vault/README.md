# Vault examples for Stellar-K8s

Use together with [`docs/vault-stellar-tutorial.md`](../../docs/vault-stellar-tutorial.md).

Files to add in your cluster (after Vault + injector are running):

- `ClusterRoleBinding` or `RoleBinding` linking the validator `ServiceAccount` to the Vault Kubernetes auth role.
- Optional `MutatingWebhookConfiguration` check: ensure the injector is not blocked by network policies on the `vault-agent` sidecar.

## Policy (illustrative)

Allow read on `secret/data/stellar/*` for the validator role. Tune to least privilege in production.

```hcl
path "secret/data/stellar/*" {
  capabilities = ["read"]
}
```

Store policies and roles via your GitOps / Vault admin workflow — not committed as static secrets here.
