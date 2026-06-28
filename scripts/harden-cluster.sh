#!/usr/bin/env bash
# scripts/harden-cluster.sh
# Verifies compliance of Stellar-K8s deployments with security benchmarks.

set -euo pipefail

# Resolve scripts library dir
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/errors.sh
source "${SCRIPT_DIR}/lib/errors.sh"

NAMESPACE="${1:-stellar}"

sk8s_step "Hardening Checks" "Running security compliance verification for namespace: $NAMESPACE"

# 1. Check Namespace PSS restricted labels
ENFORCE_LABEL=$(kubectl get ns "$NAMESPACE" -o jsonpath='{.metadata.labels.pod-security\.kubernetes\.io/enforce}' 2>/dev/null || echo "")
if [[ "$ENFORCE_LABEL" == "restricted" ]]; then
    sk8s_pass "Namespace PSS restricted enforcement: PASS"
else
    sk8s_fail "Namespace PSS restricted enforcement: FAIL" "Label 'pod-security.kubernetes.io/enforce' is not set to 'restricted'"
fi

# 2. Check running node pod security contexts
NON_ROOT=$(kubectl get pods -n "$NAMESPACE" -l app.kubernetes.io/name=stellar-node -o jsonpath='{.items[*].spec.securityContext.runAsNonRoot}' 2>/dev/null || echo "")
if [[ "$NON_ROOT" == *"true"* ]]; then
    sk8s_pass "Running pod security contexts: PASS"
else
    sk8s_fail "Running pod security contexts: FAIL" "Nodes are not configured to run as non-root"
fi

# 3. Check operator RBAC permissions
CLUSTER_ADMIN=$(kubectl get clusterrolebinding -o json | jq -r '.items[].subjects[]? | select(.name == "stellar-operator") | .name' 2>/dev/null || echo "")
if [[ -n "$CLUSTER_ADMIN" ]]; then
    sk8s_warn "Operator RBAC scope: WARNING (Operator has cluster-wide bindings, verify least privilege)"
else
    sk8s_pass "Operator RBAC scope: PASS (Operator is namespace-scoped or has no global admin access)"
fi

sk8s_step "Hardening Checks" "All security hardening checks completed"
exit 0
