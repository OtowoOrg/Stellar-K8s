# Stellar-K8s security hardening and least-privilege RBAC

This reference defines a deployable least-privilege profile for the stock `stellar-k8s run` process. The companion manifest is [`examples/security/strict-rbac.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/security/strict-rbac.yaml); the companion audit is [`examples/security/audit-rbac.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/security/audit-rbac.sh).

The reference deployment assumes:

- operator namespace: `stellar-system`
- managed workload namespace: `stellar`
- operator ServiceAccount: `stellar-operator`
- `--watch-namespace=stellar`
- mTLS, scheduled snapshots and benchmark execution are not enabled unless their separately reviewed extension permissions are added

Change names consistently in the manifest and audit environment variables.

## Security invariants

The strict profile has no RBAC wildcards. It does not allow the operator ServiceAccount to create/bind RBAC roles, create namespaces, mutate Kubernetes Secrets, use `pods/exec` or `pods/attach`, or mint ServiceAccount tokens. Validator seed material remains owned by Kubernetes/ESO/Vault/CSI.

A small read-only ClusterRole is unavoidable in the **current stock binary** because some startup/background paths ignore `--watch-namespace`. That architectural limitation is documented explicitly below rather than hidden behind a broad write-capable ClusterRole.

## Current stock-binary scope limitation

`--watch-namespace` correctly scopes the primary `StellarNode` controller, but `run_operator()` also starts several paths with cluster-wide API clients:

- startup preflight calls `Api<StellarNode>::all(...).list(...)`;
- `PeerDiscoveryManager` lists all `StellarNode` objects and looks up their Services;
- `run_benchmark_controller()` watches all `StellarBenchmark` objects and owns an all-namespaces Pod watch;
- `run_snapshot_worker()` lists all `StellarNode` objects every polling interval.

Therefore a stock process cannot be both completely cross-namespace blind and free of authorization errors merely by setting `--watch-namespace`. The strict manifest grants only the **read-only observation** needed for those loops to start: cluster-wide `list` on `StellarNode`, `list/watch` on `StellarBenchmark` and Pods, and `get` on Services. It deliberately does **not** grant cluster-wide benchmark/snapshot writes.

If your security boundary forbids even those cross-namespace reads, the correct fix is to make preflight/peer-discovery/benchmark/snapshot workers namespace-aware or independently disableable. Do not compensate by granting more cluster-wide write access.

The stock peer-discovery manager writes `stellar-peers` in `stellar-system`, so this reference uses `stellar-system` as the operator namespace.

## Exact permission map

### Managed namespace Role

| API/resource | Verbs | Why |
| --- | --- | --- |
| `stellar.org/stellarnodes` | `get,list,watch,update,patch` | Primary desired-state watch and metadata reconciliation in the managed namespace. |
| `stellar.org/stellarnodes/status` | `get,update,patch` | Publish readiness/health/reconciliation status. |
| `stellar.org/stellarnodes/finalizers` | `update` | Finalizer lifecycle. |
| `apps/deployments,statefulsets` | CRUD + `list,watch` | Node and canary workload lifecycle. |
| core `services,configmaps,persistentvolumeclaims` | CRUD + `list,watch` | Networking, configuration, alert ConfigMaps and persistent storage. |
| core `pods` | `get,list,watch,update,patch,delete` | Health/remediation and managed-pod lifecycle. |
| core `pods/log` | `get` | Diagnostics without exec access. |
| core `pods/ephemeralcontainers` | `get,update,patch` | Forensic-snapshot subresource only; remove if break-glass diagnostics are prohibited. |
| core `secrets` | `get,list,watch` | Read-only references/discovery; **no Secret writes**. |
| `networking.k8s.io/networkpolicies,ingresses` | CRUD + `list,watch` | Isolation and optional ingress. |
| `policy/poddisruptionbudgets` | CRUD + `list,watch` | Availability policy. |
| `autoscaling/horizontalpodautoscalers` | CRUD + `list,watch` | HPA when configured. |
| `autoscaling.k8s.io/verticalpodautoscalers` | `get,patch,delete` | `delete_vpa()` is called when `vpaConfig` is absent if the VPA CRD exists; `patch` supports the opt-in VPA path. |
| `external-secrets.io/externalsecrets` | `get,patch` | Apply/read the ESO object for `seedSecretSource.externalRef`; ESO writes the target Secret under its own SA. |
| `postgresql.cnpg.io/clusters,poolers` | CRUD + `list,watch` | Optional CloudNativePG managed database. Remove if the feature is excluded by policy. |
| core `events` | `create,patch` | Kubernetes event publication. |

The Role deliberately does not grant `create`/`delete` on the primary `StellarNode`, Secret mutation, `pods/exec`, `pods/attach`, RBAC writes, or namespace creation.

### Operator namespace Role

The `stellar-system` Role contains only process-runtime needs:

- `deployments get,list` for startup preflight;
- `configmaps get,list,watch,patch` for preflight, feature-flag watching, and `stellar-peers` publication;
- `leases get,list,create,update,patch` for preflight and leader election.

### Read-only runtime observer ClusterRole

The stock-binary limitation above requires:

| Resource | Verbs | Forced by |
| --- | --- | --- |
| cluster `stellar.org/stellarnodes` | `list` | preflight, peer discovery, snapshot worker |
| cluster `stellar.org/stellarbenchmarks` | `list,watch` | always-spawned benchmark controller |
| cluster Pods | `list,watch` | benchmark controller owned-Pod watch |
| cluster Services | `get` | peer discovery Service lookup |

These are read-only. The strict baseline intentionally does not authorize benchmark Pods/reports or VolumeSnapshot writes cluster-wide. Do not place active `StellarBenchmark` or scheduled-snapshot workloads outside the reviewed baseline and then treat resulting 403s as a reason to widen the role automatically.

### Named cluster-scoped exceptions

Startup preflight needs `get` on the operator Namespace. PSS and network-isolation logic need `get` on the managed Namespace, and `src/controller/pss.rs` patches its labels. The manifest therefore grants:

```yaml
resources: ["namespaces"]
resourceNames: ["stellar-system", "stellar"]
verbs: ["get"]
```

plus a separate `patch` rule restricted to `resourceNames: ["stellar"]`. Namespace creation remains denied.

Local-storage auto-detection probes only StorageClasses named `local-path` and `local-storage`, so the reference grants `get` only on those two names. If every node sets `spec.storage.storageClass`, remove that ClusterRole/binding.

## Restricted Pod Security Standards

The **node workload namespace** `stellar` is labelled `restricted` for enforce/audit/warn. The operator namespace is audit/warn only: the current Helm operator pod values do not assert every field required for `restricted` enforcement, and this issue is about enforcing PSS on namespaces that run node workloads, not hiding a chart incompatibility.

Before switching an existing workload namespace to enforcement, first run audit/warn and fix violations. Pod Security Admission validates new/updated pods; it does not rewrite existing pods.

Managed StellarNode pods already receive non-root execution, `RuntimeDefault` seccomp, `allowPrivilegeEscalation: false`, and `capabilities.drop: [ALL]` from the controller builders.

The forensic snapshot ephemeral container is a deliberate exception: it may request `NET_RAW`/`SYS_PTRACE`, which a `restricted` namespace can reject. Treat it as break-glass. Keep it disabled in the strict production profile or use a separately governed diagnostic path; do not weaken production PSS just to make diagnostics convenient.

## HashiCorp Vault validator seeds

For `seedSecretSource.vaultRef`, Stellar-K8s adds Vault Agent Injector annotations; it does not retrieve the seed value itself. Give the Vault policy only the validator path, for example KV-v2:

```hcl
path "kv/data/stellar/validators/validator-0" {
  capabilities = ["read"]
}
```

Bind the Vault Kubernetes-auth role to the **actual workload pod ServiceAccount** and workload namespace:

```bash
vault write auth/kubernetes/role/stellar-validator \
  bound_service_account_names=<validator-workload-service-account> \
  bound_service_account_namespaces=stellar \
  policies=stellar-validator \
  ttl=1h
```

Do not copy a guessed JWT audience from generic examples; audience configuration must match the deployed Kubernetes/Vault auth setup. Enable Vault audit logging, avoid list/write/delete on validator seed paths, and rotate through Vault rather than Git/Helm values.

Example:

```yaml
spec:
  validatorConfig:
    seedSecretSource:
      vaultRef:
        role: stellar-validator
        secretPath: kv/data/stellar/validators/validator-0
        secretKey: seed
        secretFileName: stellar-seed
```

## AWS Secrets Manager through ESO

Use External Secrets Operator with IRSA/workload identity. Give the **ESO ServiceAccount**, not Stellar-K8s, a narrowly scoped IAM policy:

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": ["secretsmanager:GetSecretValue", "secretsmanager:DescribeSecret"],
    "Resource": "arn:aws:secretsmanager:REGION:ACCOUNT:secret:stellar/prod/validator-*"
  }]
}
```

Reference a platform-managed `SecretStore`/`ClusterSecretStore` from the node:

```yaml
spec:
  validatorConfig:
    seedSecretSource:
      externalRef:
        name: validator-seed
        secretStoreRef:
          name: aws-secrets
          kind: ClusterSecretStore
        remoteKey: stellar/prod/validator-0
        remoteProperty: seed
        refreshInterval: 1h
```

ESO writes the resulting Kubernetes Secret. Stellar-K8s remains read-only to Secret objects in the baseline.

## Capabilities intentionally outside the baseline

Do not silently widen the baseline for optional features. Review them as separate extensions:

- `--enable-mtls` and cert-manager integration require Secret/certificate writes;
- scheduled snapshots/OCI snapshots require snapshot and/or Job permissions;
- full benchmark **execution** requires cluster-wide writes to benchmark status, Pods, reports/ConfigMaps because the current benchmark controller is all-namespaces;
- service-mesh/Istio resources require their CRD permissions;
- forensic snapshot capability can conflict with restricted PSS.

The strict manifest keeps the unavoidable all-namespaces benchmark **watch** read-only rather than granting arbitrary Pod creation across the cluster.

## Apply and audit

Apply this reference **instead of layering it on top of a broader generated ClusterRole**; RBAC is additive, so leaving the old broad binding in place defeats the exercise.

```bash
kubectl apply -f examples/security/strict-rbac.yaml
```

Run the operator with `--namespace=stellar-system --watch-namespace=stellar` (or equivalent Helm values), mTLS off, and no unreviewed optional features.

Then run:

```bash
bash examples/security/audit-rbac.sh
```

For different names, edit the static manifest names and pass matching audit values:

```bash
OPERATOR_NAMESPACE=my-operator \
MANAGED_NAMESPACE=my-stellar \
SERVICE_ACCOUNT=my-operator \
bash examples/security/audit-rbac.sh
```

The audit performs server-side manifest validation, required and forbidden `auth can-i` checks, positive/negative PSS admission tests, and an operator-log scan for RBAC errors.

## Reconciliation validation gate

Run the final proof on a disposable/test cluster with no active benchmark/snapshot extension workloads:

```bash
# Install CRDs/controller using the strict ServiceAccount and namespace settings.
# Apply a testnet StellarNode in namespace stellar, then:
kubectl get stellarnodes -n stellar -w
kubectl get deploy,statefulset,svc,pvc,networkpolicy,pdb -n stellar
kubectl logs -n stellar-system deploy/stellar-operator --since=10m \
  | grep -Ei 'forbidden|permission denied|cannot (get|list|watch|create|update|patch|delete)' \
  && echo 'RBAC FAILURE' || echo 'no RBAC errors found'
bash examples/security/audit-rbac.sh
```

A clean gate means the primary reconciliation succeeds, the stock background watches do not emit RBAC errors, restricted PSS accepts compliant node pods and rejects privileged pods, and high-risk permissions remain denied.

## Security-review rejection criteria

Reject the profile if it introduces RBAC wildcards; Secret write verbs in the baseline; RBAC role/binding writes; namespace creation; cross-namespace Secret reads; `pods/exec`, `pods/attach`, or `serviceaccounts/token`; unrestricted Namespace patching; cluster-wide workload writes merely to silence optional workers; or weaker node-workload PSS without an explicit break-glass decision.

The permission map is grounded in `charts/stellar-operator/templates/rbac.yaml`, `src/commands/operator.rs`, `src/preflight.rs`, `src/controller/reconciler.rs`, `src/controller/resources.rs`, `src/controller/pss.rs`, `src/controller/network_isolation.rs`, `src/controller/kms_secret.rs`, `src/controller/peer_discovery.rs`, `src/controller/benchmark/reconciler.rs`, and `src/controller/snapshot_worker.rs`. Re-audit this document when those paths change.
