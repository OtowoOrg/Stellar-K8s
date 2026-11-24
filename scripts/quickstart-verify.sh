#!/usr/bin/env bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
# scripts/quickstart-verify.sh
#
# End-to-end quickstart verification for Stellar-K8s.
# Mirrors every command shown in the README "Quick Start > Kubernetes" section
# so that documentation rot is caught automatically.
#
# What it does:
#   1. Creates a local kind cluster
#   2. Builds the operator Docker image via make docker-build-ci
#      (container-native build; avoids host/glibc vs bookworm mismatches)
#   3. Loads the image into kind
#   4. Installs the operator via Helm (local chart, not the remote repo)
#   5. Applies the sample StellarNode manifest
#   6. Verifies the operator pod becomes Ready
#   7. Verifies the CRD was installed successfully
#   8. Cleans up the kind cluster
#
# Usage:
#   bash scripts/quickstart-verify.sh           # full run (creates + destroys cluster)
#   SKIP_CLEANUP=1 bash scripts/quickstart-verify.sh  # keep cluster after run
#   CLUSTER_NAME=my-test bash scripts/quickstart-verify.sh
#
# Requirements:
#   - kind    (https://kind.sigs.k8s.io)
#   - kubectl (https://kubernetes.io/docs/tasks/tools/)
#   - helm    (https://helm.sh/docs/intro/install/)
#   - docker  (https://docs.docker.com/get-docker/)
#   - cargo   (https://rustup.rs/)

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
CLUSTER_NAME="${CLUSTER_NAME:-stellar-quickstart-verify}"
NAMESPACE="${NAMESPACE:-stellar-system}"
IMAGE_TAG="quickstart-verify-$(date +%s)"
SKIP_CLEANUP="${SKIP_CLEANUP:-0}"
TIMEOUT="${TIMEOUT:-120s}"

# ── Colour helpers ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

step()  { echo -e "\n${BOLD}── $* ──${RESET}"; }
pass()  { echo -e "  ${GREEN}✓${RESET} $*"; }
fail()  { echo -e "  ${RED}✗${RESET} $*"; }
warn()  { echo -e "  ${YELLOW}⚠${RESET}  $*"; }

FAILURES=0

# ── Cleanup trap ──────────────────────────────────────────────────────────────
cleanup() {
  if [[ "$SKIP_CLEANUP" == "1" ]]; then
    warn "SKIP_CLEANUP=1 — leaving cluster '${CLUSTER_NAME}' running"
    echo "  To delete manually: kind delete cluster --name ${CLUSTER_NAME}"
    return
  fi
  step "Cleanup"
  echo "  Deleting kind cluster '${CLUSTER_NAME}'..."
  kind delete cluster --name "${CLUSTER_NAME}" 2>/dev/null || true
  pass "Cluster deleted"
}
trap cleanup EXIT

# ── Prerequisite checks ───────────────────────────────────────────────────────
step "Checking prerequisites"

check_cmd() {
  local cmd="$1"
  local install_hint="${2:-}"
  if command -v "$cmd" >/dev/null 2>&1; then
    pass "$cmd found ($(command -v "$cmd"))"
  else
    fail "$cmd not found${install_hint:+ — $install_hint}"
    FAILURES=$((FAILURES + 1))
  fi
}

# These mirror the README prerequisites section exactly
check_cmd "kind"    "Install: https://kind.sigs.k8s.io/docs/user/quick-start/#installation"
check_cmd "kubectl" "Install: https://kubernetes.io/docs/tasks/tools/"
check_cmd "helm"    "Install: https://helm.sh/docs/intro/install/"
check_cmd "docker"  "Install: https://docs.docker.com/get-docker/"
check_cmd "cargo"   "Install: https://rustup.rs/"

if [[ "$FAILURES" -gt 0 ]]; then
  echo -e "\n${RED}${BOLD}Cannot proceed: $FAILURES prerequisite(s) missing.${RESET}"
  exit 1
fi

# ── Step 1: Create kind cluster ───────────────────────────────────────────────
# Mirrors README: "kubectl configured" prerequisite
step "Step 1: Create kind cluster '${CLUSTER_NAME}'"
if kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
  warn "Cluster '${CLUSTER_NAME}' already exists — reusing"
else
  kind create cluster --name "${CLUSTER_NAME}" --wait "${TIMEOUT}"
  pass "Cluster created"
fi

# Point kubectl at the new cluster
kubectl config use-context "kind-${CLUSTER_NAME}"
pass "kubectl context set to kind-${CLUSTER_NAME}"

# ── Step 2: Build operator Docker image (container-native) ────────────────────
# Prefer `make docker-build-ci` over host `make build` + `runtime-local`: GitHub
# runners (and many local hosts) ship a newer glibc than debian:bookworm-slim,
# which causes CrashLoopBackOff ("GLIBC_2.38/2.39 not found") when host binaries
# are copied into the bookworm runtime image.
step "Step 2: Build operator Docker image (make docker-build-ci)"
IMAGE_NAME=stellar-operator IMAGE_TAG="${IMAGE_TAG}" make docker-build-ci
pass "Docker image built"

# ── Step 3: Load image into kind ─────────────────────────────────────────────
step "Step 3: Load image into kind cluster"
kind load docker-image "stellar-operator:${IMAGE_TAG}" --name "${CLUSTER_NAME}"
pass "Image loaded into kind"

# ── Step 4: Install CRD ──────────────────────────────────────────────────────
# Mirrors README: "kubectl apply -f config/crd/stellarnode-crd.yaml"
step "Step 4: Install StellarNode CRD"
kubectl apply -f config/crd/stellarnode-crd.yaml
pass "CRD applied"

# Verify the CRD was registered
if kubectl get crd stellarnodes.stellar.org >/dev/null 2>&1; then
  pass "CRD 'stellarnodes.stellar.org' confirmed registered"
else
  fail "CRD 'stellarnodes.stellar.org' NOT found after apply"
  FAILURES=$((FAILURES + 1))
fi

# Helm also renders CRDs from templates/; adopt the kubectl-applied CRD so the
# subsequent helm install can take ownership without metadata conflicts.
kubectl label --overwrite crd stellarnodes.stellar.org \
  app.kubernetes.io/managed-by=Helm >/dev/null
kubectl annotate --overwrite crd stellarnodes.stellar.org \
  meta.helm.sh/release-name=stellar-operator \
  meta.helm.sh/release-namespace="${NAMESPACE}" >/dev/null
pass "CRD labeled for Helm release ownership"

# ── Step 5: Create namespace ──────────────────────────────────────────────────
# Mirrors README: "--namespace stellar-system --create-namespace"
step "Step 5: Create namespace '${NAMESPACE}'"
kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -
pass "Namespace '${NAMESPACE}' ready"

# ── Step 6: Install operator via Helm ────────────────────────────────────────
# Mirrors README Helm install command (using local chart instead of remote repo)
step "Step 6: Install operator via Helm"
if ! helm upgrade --install stellar-operator charts/stellar-operator \
  --namespace "${NAMESPACE}" \
  --set image.tag="${IMAGE_TAG}" \
  --set image.repository="stellar-operator" \
  --set image.pullPolicy=Never \
  --set hooks.preInstall.enabled=false \
  --set hooks.preUpgrade.enabled=false \
  --set sidecar.enabled=false \
  --set webhook.enabled=false \
  --disable-openapi-validation \
  --wait \
  --timeout "${TIMEOUT}"; then
  fail "Helm install failed — collecting diagnostics before exit"
  kubectl get pods -n "${NAMESPACE}" -o wide || true
  kubectl describe deployment -n "${NAMESPACE}" stellar-operator || true
  kubectl get events -n "${NAMESPACE}" --sort-by='.lastTimestamp' | tail -40 || true
  kubectl logs -n "${NAMESPACE}" -l "app.kubernetes.io/name=stellar-operator" --all-containers --tail=200 || true
  exit 1
fi
pass "Helm install complete"

# ── Step 7: Verify operator pod is Running/Ready ──────────────────────────────
step "Step 7: Verify operator deployment is Ready"
echo "  Waiting for Deployment rollout..."
kubectl rollout status deployment/stellar-operator \
  --namespace "${NAMESPACE}" \
  --timeout "${TIMEOUT}"
pass "Deployment rolled out successfully"

# Verify at least one pod is Running
RUNNING_PODS=$(kubectl get pods -n "${NAMESPACE}" \
  -l "app.kubernetes.io/name=stellar-operator" \
  --field-selector=status.phase=Running \
  -o name 2>/dev/null | wc -l)
if [[ "$RUNNING_PODS" -gt 0 ]]; then
  pass "${RUNNING_PODS} operator pod(s) are Running"
else
  fail "No Running operator pods found in namespace '${NAMESPACE}'"
  FAILURES=$((FAILURES + 1))
  kubectl get pods -n "${NAMESPACE}" || true
fi

# ── Step 8: Apply sample StellarNode ─────────────────────────────────────────
# Mirrors README: "kubectl apply -f validator.yaml"
step "Step 8: Apply sample StellarNode manifest"
kubectl apply -f config/samples/test-stellarnode.yaml
pass "Sample StellarNode applied"

# Verify the resource was accepted by the API server
if kubectl get stellarnodes -n "${NAMESPACE}" >/dev/null 2>&1; then
  NODE_COUNT=$(kubectl get stellarnodes -n "${NAMESPACE}" --no-headers 2>/dev/null | wc -l)
  pass "${NODE_COUNT} StellarNode resource(s) found"
else
  warn "Could not list StellarNodes — checking without namespace filter"
  kubectl get stellarnodes --all-namespaces 2>/dev/null || true
fi

# ── Step 9: Helm lint of local chart ────────────────────────────────────────
# Mirrors README dev section: "make helm-lint"
step "Step 9: Helm lint (make helm-lint)"
helm lint charts/stellar-operator
pass "helm lint passed"

# ── Final summary ─────────────────────────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════"
if [[ "$FAILURES" -eq 0 ]]; then
  echo -e "${GREEN}${BOLD}✓ All quickstart verification steps passed!${RESET}"
  echo ""
  echo "  The following README commands were verified:"
  echo "    • kind create cluster"
  echo "    • make docker-build-ci"
  echo "    • kind load docker-image"
  echo "    • kubectl apply -f config/crd/stellarnode-crd.yaml"
  echo "    • helm upgrade --install stellar-operator"
  echo "    • kubectl apply -f config/samples/test-stellarnode.yaml"
  echo "    • make helm-lint"
else
  echo -e "${RED}${BOLD}✗ $FAILURES verification step(s) FAILED${RESET}"
  echo ""
  echo "  Review the output above and fix failing steps."
  exit 1
fi
