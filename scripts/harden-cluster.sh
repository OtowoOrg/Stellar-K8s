#!/usr/bin/env bash
# scripts/harden-cluster.sh
# Verifies compliance of Stellar-K8s deployments with security benchmarks.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0;30m'

NAMESPACE="${1:-stellar}"

echo "=== Running Security Hardening Checks for namespace: $NAMESPACE ==="

# 1. Check Namespace PSS restricted labels
echo -n "Checking Namespace PSS restricted enforcement... "
ENFORCE_LABEL=$(kubectl get ns "$NAMESPACE" -o jsonpath='{.metadata.labels.pod-security\.kubernetes\.io/enforce}' 2>/dev/null || echo "")
if [[ "$ENFORCE_LABEL" == "restricted" ]]; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL${NC} (Label 'pod-security.kubernetes.io/enforce' is not set to 'restricted')"
    exit 1
fi

# 2. Check running node pod security contexts
echo -n "Verifying running pod security contexts... "
NON_ROOT=$(kubectl get pods -n "$NAMESPACE" -l app.kubernetes.io/name=stellar-node -o jsonpath='{.items[*].spec.securityContext.runAsNonRoot}' 2>/dev/null || echo "")
if [[ "$NON_ROOT" == *"true"* ]]; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL${NC} (Nodes are not configured to run as non-root)"
    exit 1
fi

# 3. Check operator RBAC permissions
echo -n "Checking operator RBAC scope... "
CLUSTER_ADMIN=$(kubectl get clusterrolebinding -o json | jq -r '.items[].subjects[]? | select(.name == "stellar-operator") | .name' 2>/dev/null || echo "")
if [[ -n "$CLUSTER_ADMIN" ]]; then
    echo -e "${RED}WARNING${NC} (Operator has cluster-wide bindings, verify least privilege)"
else
    echo -e "${GREEN}PASS${NC} (Operator is namespace-scoped or has no global admin access)"
fi

echo "=== All Security Hardening Checks Completed ==="
exit 0
