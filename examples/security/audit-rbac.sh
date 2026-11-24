#!/usr/bin/env bash
set -euo pipefail

OPERATOR_NAMESPACE="${OPERATOR_NAMESPACE:-stellar-system}"
MANAGED_NAMESPACE="${MANAGED_NAMESPACE:-stellar}"
SERVICE_ACCOUNT="${SERVICE_ACCOUNT:-stellar-operator}"
DEPLOYMENT_NAME="${DEPLOYMENT_NAME:-stellar-operator}"
MANIFEST="${MANIFEST:-examples/security/strict-rbac.yaml}"
AS="system:serviceaccount:${OPERATOR_NAMESPACE}:${SERVICE_ACCOUNT}"

fail=0
pass() { printf 'PASS  %s\n' "$*"; }
fail_check() { printf 'FAIL  %s\n' "$*" >&2; fail=1; }

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "required command not found: $1" >&2
    exit 2
  }
}
need kubectl

can_ns() {
  kubectl auth can-i "$1" "$2" -n "$3" --as="$AS" 2>/dev/null
}

expect_yes_ns() {
  if [[ "$(can_ns "$1" "$2" "$3")" == "yes" ]]; then
    pass "$1 $2 in $3"
  else
    fail_check "expected ALLOW: $1 $2 in $3"
  fi
}

expect_no_ns() {
  if [[ "$(can_ns "$1" "$2" "$3")" == "no" ]]; then
    pass "denied $1 $2 in $3"
  else
    fail_check "expected DENY: $1 $2 in $3"
  fi
}

expect_yes_all() {
  if kubectl auth can-i "$1" "$2" --all-namespaces --as="$AS" 2>/dev/null | grep -qx yes; then
    pass "$1 $2 across all namespaces"
  else
    fail_check "expected ALLOW: $1 $2 across all namespaces"
  fi
}

expect_no_cluster() {
  if kubectl auth can-i "$1" "$2" --as="$AS" 2>/dev/null | grep -qx no; then
    pass "denied cluster-scope $1 $2"
  else
    fail_check "expected DENY: cluster-scope $1 $2"
  fi
}

echo "== Server-side manifest validation =="
kubectl apply --dry-run=server -f "$MANIFEST" >/dev/null
pass "strict RBAC manifest accepted by API server"

echo "== Managed-namespace reconciliation permissions =="
expect_yes_ns get stellarnodes.stellar.org "$MANAGED_NAMESPACE"
expect_yes_ns list stellarnodes.stellar.org "$MANAGED_NAMESPACE"
if kubectl auth can-i patch stellarnodes.stellar.org --subresource=status -n "$MANAGED_NAMESPACE" --as="$AS" 2>/dev/null | grep -qx yes; then
  pass "patch stellarnodes.stellar.org/status in $MANAGED_NAMESPACE"
else
  fail_check "expected ALLOW: patch stellarnodes.stellar.org/status in $MANAGED_NAMESPACE"
fi
expect_yes_ns create deployments.apps "$MANAGED_NAMESPACE"
expect_yes_ns delete statefulsets.apps "$MANAGED_NAMESPACE"
expect_yes_ns get secrets "$MANAGED_NAMESPACE"
expect_yes_ns create networkpolicies.networking.k8s.io "$MANAGED_NAMESPACE"
expect_yes_ns create poddisruptionbudgets.policy "$MANAGED_NAMESPACE"
expect_yes_ns create events "$MANAGED_NAMESPACE"

echo "== Operator-namespace runtime permissions =="
expect_yes_ns list deployments.apps "$OPERATOR_NAMESPACE"
expect_yes_ns list configmaps "$OPERATOR_NAMESPACE"
expect_yes_ns watch configmaps "$OPERATOR_NAMESPACE"
expect_yes_ns patch configmaps "$OPERATOR_NAMESPACE"
expect_yes_ns list leases.coordination.k8s.io "$OPERATOR_NAMESPACE"
expect_yes_ns patch leases.coordination.k8s.io "$OPERATOR_NAMESPACE"

echo "== Stock-binary read-only cluster observation =="
expect_yes_all list stellarnodes.stellar.org
expect_yes_all list stellarbenchmarks.stellar.org
expect_yes_all watch stellarbenchmarks.stellar.org
expect_yes_all list pods
expect_yes_all watch pods
expect_yes_all get services

if kubectl auth can-i get namespaces/"$OPERATOR_NAMESPACE" --as="$AS" 2>/dev/null | grep -qx yes; then
  pass "get namespace/$OPERATOR_NAMESPACE"
else
  fail_check "expected ALLOW: get namespace/$OPERATOR_NAMESPACE"
fi
if kubectl auth can-i get namespaces/"$MANAGED_NAMESPACE" --as="$AS" 2>/dev/null | grep -qx yes; then
  pass "get namespace/$MANAGED_NAMESPACE"
else
  fail_check "expected ALLOW: get namespace/$MANAGED_NAMESPACE"
fi
if kubectl auth can-i patch namespaces/"$MANAGED_NAMESPACE" --as="$AS" 2>/dev/null | grep -qx yes; then
  pass "patch namespace/$MANAGED_NAMESPACE"
else
  fail_check "expected ALLOW: patch namespace/$MANAGED_NAMESPACE"
fi

echo "== High-risk permissions that must remain denied =="
expect_no_ns delete secrets "$MANAGED_NAMESPACE"
expect_no_ns create secrets "$MANAGED_NAMESPACE"
expect_no_ns create pods "$OPERATOR_NAMESPACE"
expect_no_cluster create clusterroles.rbac.authorization.k8s.io
expect_no_cluster create clusterrolebindings.rbac.authorization.k8s.io
expect_no_cluster create namespaces
expect_no_ns get secrets kube-system

if kubectl auth can-i create pods --subresource=exec -n "$MANAGED_NAMESPACE" --as="$AS" 2>/dev/null | grep -qx no; then
  pass "denied create pods/exec in $MANAGED_NAMESPACE"
else
  fail_check "expected DENY: create pods/exec in $MANAGED_NAMESPACE"
fi
if kubectl auth can-i create serviceaccounts --subresource=token -n "$MANAGED_NAMESPACE" --as="$AS" 2>/dev/null | grep -qx no; then
  pass "denied create serviceaccounts/token in $MANAGED_NAMESPACE"
else
  fail_check "expected DENY: create serviceaccounts/token in $MANAGED_NAMESPACE"
fi
if kubectl auth can-i patch namespaces/kube-system --as="$AS" 2>/dev/null | grep -qx no; then
  pass "denied patch namespaces/kube-system"
else
  fail_check "expected DENY: patch namespaces/kube-system"
fi

# The always-spawned benchmark watcher must remain read-only in the strict profile.
if kubectl auth can-i create pods --all-namespaces --as="$AS" 2>/dev/null | grep -qx no; then
  pass "denied cluster-wide Pod creation"
else
  fail_check "expected DENY: cluster-wide Pod creation"
fi
if kubectl auth can-i patch stellarbenchmarks.stellar.org --subresource=status --all-namespaces --as="$AS" 2>/dev/null | grep -qx no; then
  pass "denied cluster-wide StellarBenchmark status mutation"
else
  fail_check "expected DENY: cluster-wide StellarBenchmark status mutation"
fi

echo "== Pod Security Admission =="
cat <<'YAML' | kubectl apply --dry-run=server -n "$MANAGED_NAMESPACE" -f - >/dev/null
apiVersion: v1
kind: Pod
metadata:
  name: stellar-pss-positive-test
spec:
  restartPolicy: Never
  containers:
    - name: test
      image: registry.k8s.io/pause:3.10
      securityContext:
        allowPrivilegeEscalation: false
        runAsNonRoot: true
        capabilities:
          drop: ["ALL"]
        seccompProfile:
          type: RuntimeDefault
  securityContext:
    runAsNonRoot: true
    seccompProfile:
      type: RuntimeDefault
YAML
pass "restricted-compatible pod admitted by server dry-run"

set +e
pss_bad_output="$({ cat <<'YAML'
apiVersion: v1
kind: Pod
metadata:
  name: stellar-pss-negative-test
spec:
  containers:
    - name: test
      image: registry.k8s.io/pause:3.10
      securityContext:
        privileged: true
YAML
} | kubectl apply --dry-run=server -n "$MANAGED_NAMESPACE" -f - 2>&1)"
pss_bad_rc=$?
set -e
if [[ $pss_bad_rc -ne 0 ]] && grep -qiE 'podsecurity|restricted|privileged' <<<"$pss_bad_output"; then
  pass "privileged pod rejected by Pod Security Admission"
else
  fail_check "privileged pod was not rejected by restricted Pod Security Admission"
fi

echo "== Operator reconciliation log gate =="
if kubectl get deployment -n "$OPERATOR_NAMESPACE" "$DEPLOYMENT_NAME" >/dev/null 2>&1; then
  if kubectl logs -n "$OPERATOR_NAMESPACE" deployment/"$DEPLOYMENT_NAME" --since=5m 2>&1 \
      | grep -Eqi 'forbidden|permission denied|cannot (get|list|watch|create|update|patch|delete)'; then
    fail_check "operator logs contain RBAC/permission errors in the last 5 minutes"
  else
    pass "no RBAC/permission errors in operator logs in the last 5 minutes"
  fi
else
  echo "SKIP  deployment/$DEPLOYMENT_NAME not found; set DEPLOYMENT_NAME for the log gate"
fi

if [[ $fail -ne 0 ]]; then
  echo "RBAC audit FAILED" >&2
  exit 1
fi

echo "RBAC audit PASSED"
