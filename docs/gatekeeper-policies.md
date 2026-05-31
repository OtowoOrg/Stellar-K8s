# OPA Gatekeeper Policies for StellarNode

This guide covers installing OPA Gatekeeper, applying the StellarNode admission policies, testing them, and verifying that the operator's reconciliation loop continues to work correctly.

---

## Prerequisites

- Kubernetes cluster (1.25+)
- `kubectl` configured with cluster-admin privileges
- **OPA Gatekeeper v3.13 or later**

### Install Gatekeeper

```bash
kubectl apply -f https://raw.githubusercontent.com/open-policy-agent/gatekeeper/v3.13.0/deploy/gatekeeper.yaml
```

Wait for Gatekeeper to be ready:

```bash
kubectl -n gatekeeper-system rollout status deployment/gatekeeper-controller-manager
kubectl -n gatekeeper-system rollout status deployment/gatekeeper-audit
```

---

## Install the StellarNode Policies

Apply the Gatekeeper `Config` resource first to exclude system namespaces from all policy evaluation:

```bash
kubectl apply -f config/manifests/gatekeeper/gatekeeper-config.yaml
```

Apply all three ConstraintTemplates:

```bash
kubectl apply -f config/manifests/gatekeeper/resource-limits-template.yaml
kubectl apply -f config/manifests/gatekeeper/approved-registries-template.yaml
kubectl apply -f config/manifests/gatekeeper/required-labels-template.yaml
```

Wait for the ConstraintTemplates to be fully reconciled before applying Constraints (the CRDs they define must exist first):

```bash
kubectl wait --for=condition=Ready constrainttemplate/resourcelimits --timeout=60s
kubectl wait --for=condition=Ready constrainttemplate/approvedregistries --timeout=60s
kubectl wait --for=condition=Ready constrainttemplate/requiredlabels --timeout=60s
```

Apply all three Constraints:

```bash
kubectl apply -f config/manifests/gatekeeper/resource-limits-constraint.yaml
kubectl apply -f config/manifests/gatekeeper/approved-registries-constraint.yaml
kubectl apply -f config/manifests/gatekeeper/required-labels-constraint.yaml
```

Or apply the entire directory at once (after ConstraintTemplates are ready):

```bash
kubectl apply -f config/manifests/gatekeeper/
```

Verify all constraints are active:

```bash
kubectl get constraint
```

Expected output:

```
NAME                              ENFORCEMENT-ACTION   TOTAL-VIOLATIONS
stellarnode-approved-registries   deny                 0
stellarnode-required-labels       deny                 0
stellarnode-resource-limits       deny                 0
```

---

## Testing the Policies

### Policy 1: Resource Limits

#### Non-compliant: CPU exceeds maximum

```yaml
# test-resource-limits-bad-cpu.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-bad-cpu
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: test
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
  resources:
    limits:
      cpu: "8"        # exceeds max_cpu of "4"
      memory: "4Gi"
```

```bash
kubectl apply -f test-resource-limits-bad-cpu.yaml
```

Expected denial:

```
Error from server (Forbidden): error when creating "test-resource-limits-bad-cpu.yaml":
admission webhook "validation.gatekeeper.sh" denied the request:
[stellarnode-resource-limits] StellarNode 'test-bad-cpu' cpu limit '8' exceeds maximum '4'
```

#### Non-compliant: Memory exceeds maximum

```yaml
# test-resource-limits-bad-memory.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-bad-memory
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: test
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
  resources:
    limits:
      cpu: "2"
      memory: "16Gi"  # exceeds max_memory of "8Gi"
```

```bash
kubectl apply -f test-resource-limits-bad-memory.yaml
```

Expected denial:

```
Error from server (Forbidden): ...
[stellarnode-resource-limits] StellarNode 'test-bad-memory' memory limit '16Gi' exceeds maximum '8Gi'
```

#### Non-compliant: Missing resource limits entirely

```yaml
# test-resource-limits-missing.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-no-resources
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: test
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
```

```bash
kubectl apply -f test-resource-limits-missing.yaml
```

Expected denial:

```
Error from server (Forbidden): ...
[stellarnode-resource-limits] StellarNode 'test-no-resources' must specify spec.resources.limits
```

#### Compliant: Within resource limits

```yaml
# test-resource-limits-good.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-good-resources
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: test
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
```

```bash
kubectl apply -f test-resource-limits-good.yaml
```

Expected result: `stellarnode.stellar.org/test-good-resources created`

---

### Policy 2: Approved Image Registries

#### Non-compliant: Unapproved registry

```yaml
# test-registry-bad.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-bad-registry
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: test
spec:
  version: "docker.io/untrusted/stellar-node:latest"  # not in approved list
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
```

```bash
kubectl apply -f test-registry-bad.yaml
```

Expected denial:

```
Error from server (Forbidden): ...
[stellarnode-approved-registries] StellarNode 'test-bad-registry' image registry
'docker.io/untrusted/stellar-node:latest' is not in the approved list:
["docker.io/stellar", "ghcr.io/stellar", "registry.stellar.org"]
```

#### Compliant: Approved registry

```yaml
# test-registry-good.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-good-registry
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: test
spec:
  version: "ghcr.io/stellar/stellar-node:v1.2.3"
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
```

```bash
kubectl apply -f test-registry-good.yaml
```

Expected result: `stellarnode.stellar.org/test-good-registry created`

---

### Policy 3: Required Labels

#### Non-compliant: Missing required labels

```yaml
# test-labels-bad.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-bad-labels
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    # missing stellar.org/team and stellar.org/environment
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
```

```bash
kubectl apply -f test-labels-bad.yaml
```

Expected denial:

```
Error from server (Forbidden): ...
[stellarnode-required-labels] StellarNode 'test-bad-labels' is missing required labels:
["stellar.org/team", "stellar.org/environment"]
```

#### Non-compliant: No labels at all

```yaml
# test-labels-none.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-no-labels
  namespace: default
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
```

```bash
kubectl apply -f test-labels-none.yaml
```

Expected denial:

```
Error from server (Forbidden): ...
[stellarnode-required-labels] StellarNode 'test-no-labels' is missing required labels:
["app.kubernetes.io/part-of", "stellar.org/team", "stellar.org/environment"]
```

#### Compliant: All required labels present

```yaml
# test-labels-good.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-good-labels
  namespace: default
  labels:
    app.kubernetes.io/part-of: stellar
    stellar.org/team: platform
    stellar.org/environment: production
spec:
  version: "ghcr.io/stellar/stellar-node:latest"
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
```

```bash
kubectl apply -f test-labels-good.yaml
```

Expected result: `stellarnode.stellar.org/test-good-labels created`

---

## Verifying Operator Reconciliation

After applying the policies, confirm the operator continues to reconcile StellarNode resources normally.

### Check StellarNode status

```bash
kubectl get stellarnodes --all-namespaces
```

Expected output shows nodes in a `Ready` or `Running` state — the same as before policies were applied:

```
NAMESPACE   NAME              STATUS    AGE
stellar     my-stellar-node   Running   5m
```

### Check operator pod logs

```bash
kubectl -n stellar logs -l app=stellar-operator --tail=50
```

Look for normal reconciliation messages. There should be no admission webhook denial errors in the operator logs. Example healthy output:

```
INFO  stellar_operator::controller > reconciling StellarNode my-stellar-node
INFO  stellar_operator::controller > StellarNode my-stellar-node reconciled successfully
```

If you see errors like `admission webhook "validation.gatekeeper.sh" denied the request`, the operator's namespace exemption may not be configured correctly — see the [Operator Exemption](#operator-exemption) section below.

---

## Auditing Existing Resources

Gatekeeper's audit controller periodically evaluates all existing resources against active Constraints and records violations. No action is required to enable audit mode — it runs automatically.

### View current violations

```bash
kubectl get constraint
```

The `TOTAL-VIOLATIONS` column shows how many existing resources violate each policy.

### Inspect violation details

```bash
kubectl describe constraint stellarnode-resource-limits
kubectl describe constraint stellarnode-approved-registries
kubectl describe constraint stellarnode-required-labels
```

The `status.violations` section lists each violating resource:

```yaml
status:
  violations:
  - enforcementAction: deny
    group: stellar.org
    kind: StellarNode
    message: "StellarNode 'legacy-node' cpu limit '10' exceeds maximum '4'"
    name: legacy-node
    namespace: production
    version: v1alpha1
```

### Adjust audit interval

The default audit interval is 60 seconds. To change it, edit the Gatekeeper controller deployment:

```bash
kubectl -n gatekeeper-system edit deployment gatekeeper-audit
# Add or modify: --audit-interval=30 (seconds)
```

---

## Operator Exemption

The `stellar-operator` service account must never be blocked by the policies it governs. Exemption is implemented at two levels.

### Namespace exclusion (default)

All three Constraint manifests include `spec.match.excludedNamespaces: [stellar]`. Resources in the `stellar` namespace — where the `stellar-operator` service account runs — are excluded from policy evaluation entirely.

This is sufficient when the operator only creates StellarNodes in the `stellar` namespace.

### User-level exemption (cross-namespace deployments)

If the operator creates StellarNodes in namespaces other than `stellar`, namespace exclusion alone is not enough. Add the `stellar-operator` service account to the Gatekeeper `Config` `exemptUsers` list:

```yaml
# config/manifests/gatekeeper/gatekeeper-config.yaml (updated)
apiVersion: config.gatekeeper.sh/v1alpha1
kind: Config
metadata:
  name: config
  namespace: gatekeeper-system
spec:
  match:
    - excludedNamespaces:
        - kube-system
        - gatekeeper-system
      processes:
        - "*"
  exemptUsers:
    - "system:serviceaccount:stellar:stellar-operator"
```

Apply the updated config:

```bash
kubectl apply -f config/manifests/gatekeeper/gatekeeper-config.yaml
```

> If the operator's service account name or namespace is customized, update the `exemptUsers` entry accordingly. The format is `system:serviceaccount:<namespace>:<service-account-name>`. The default is `system:serviceaccount:stellar:stellar-operator`.

### Updating Constraint excludedNamespaces

If the operator namespace is not `stellar`, update the `spec.match.excludedNamespaces` field in each Constraint manifest before applying:

```yaml
spec:
  match:
    excludedNamespaces:
      - <your-operator-namespace>  # replace "stellar" with your namespace
```

---

## Strict Enforcement Mode

By default, Gatekeeper's `ValidatingWebhookConfiguration` is installed with `failurePolicy: Ignore`. This means that if the Gatekeeper pod is unavailable (e.g., during an upgrade or outage), admission requests are allowed through rather than blocked.

To change to strict enforcement — where all admission requests are denied if Gatekeeper is unreachable — patch the webhook configuration:

```bash
kubectl patch validatingwebhookconfiguration gatekeeper-validating-webhook-configuration \
  --type='json' \
  -p='[{"op":"replace","path":"/webhooks/0/failurePolicy","value":"Fail"},
       {"op":"replace","path":"/webhooks/1/failurePolicy","value":"Fail"}]'
```

> **Warning:** Setting `failurePolicy: Fail` means any Gatekeeper downtime will block all StellarNode CREATE and UPDATE operations cluster-wide. Ensure Gatekeeper is running with sufficient replicas and pod disruption budgets before enabling strict mode.

To revert to the default:

```bash
kubectl patch validatingwebhookconfiguration gatekeeper-validating-webhook-configuration \
  --type='json' \
  -p='[{"op":"replace","path":"/webhooks/0/failurePolicy","value":"Ignore"},
       {"op":"replace","path":"/webhooks/1/failurePolicy","value":"Ignore"}]'
```
