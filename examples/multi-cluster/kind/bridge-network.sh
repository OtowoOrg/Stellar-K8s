#!/usr/bin/env bash
#
# bridge-network.sh - Provision two kind clusters connected over a virtual
# bridge network and deploy the multi-cluster Stellar-K8s architecture.
#
# This script:
#   1. Creates kind-cluster-a and kind-cluster-b.
#   2. Joins both to a shared Docker bridge network.
#   3. Installs the Stellar-K8s operator, cert-manager, ExternalDNS, and Istio.
#   4. Applies the Cluster A and Cluster B manifests.
#   5. Verifies cross-cluster peer connectivity on port 11625.
#
# Prerequisites:
#   - kind, kubectl, docker, helm installed
#   - A shared CA secret (see examples/multi-cluster/mtls/ca.yaml)
#
# Usage:
#   ./bridge-network.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
BRIDGE_NET="stellar-bridge"

echo "==> Creating shared Docker bridge network: ${BRIDGE_NET}"
docker network create "${BRIDGE_NET}" 2>/dev/null || true

echo "==> Creating Cluster A (Primary)"
kind create cluster --name cluster-a --config "${SCRIPT_DIR}/cluster-a.yaml"
docker network connect "${BRIDGE_NET}" kind-control-plane 2>/dev/null || true
docker network connect "${BRIDGE_NET}" kind-worker 2>/dev/null || true

echo "==> Creating Cluster B (Secondary)"
kind create cluster --name cluster-b --config "${SCRIPT_DIR}/cluster-b.yaml"
docker network connect "${BRIDGE_NET}" kind-control-plane 2>/dev/null || true
docker network connect "${BRIDGE_NET}" kind-worker 2>/dev/null || true

echo "==> Installing Stellar-K8s operator (Cluster A)"
kubectl --context kind-cluster-a apply -f "${ROOT_DIR}/config/crd"
kubectl --context kind-cluster-a apply -f "${ROOT_DIR}/config/manager"

echo "==> Installing Stellar-K8s operator (Cluster B)"
kubectl --context kind-cluster-b apply -f "${ROOT_DIR}/config/crd"
kubectl --context kind-cluster-b apply -f "${ROOT_DIR}/config/manager"

echo "==> Installing cert-manager (both clusters)"
for ctx in kind-cluster-a kind-cluster-b; do
  kubectl --context "${ctx}" apply -f \
    https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.crds.yaml
  helm --kube-context "${ctx}" repo add jetstack https://charts.jetstack.io
  helm --kube-context "${ctx}" repo update
  helm --kube-context "${ctx}" upgrade --install cert-manager jetstack/cert-manager \
    --namespace cert-manager --create-namespace --version v1.13.0
done

echo "==> Installing ExternalDNS (both clusters)"
for ctx in kind-cluster-a kind-cluster-b; do
  kubectl --context "${ctx}" apply -f "${ROOT_DIR}/examples/multi-cluster/${ctx#kind-}/external-dns.yaml"
done

echo "==> Installing Istio (both clusters)"
for ctx in kind-cluster-a kind-cluster-b; do
  helm --kube-context "${ctx}" repo add istio https://istio-release.storage.googleapis.com/charts
  helm --kube-context "${ctx}" repo update
  helm --kube-context "${ctx}" upgrade --install istio-base istio/base -n istio-system --create-namespace
  helm --kube-context "${ctx}" upgrade --install istiod istio/istiod -n istio-system --wait
done

echo "==> Applying mTLS manifests (both clusters)"
for ctx in kind-cluster-a kind-cluster-b; do
  kubectl --context "${ctx}" apply -f "${ROOT_DIR}/examples/multi-cluster/mtls/ca.yaml"
  kubectl --context "${ctx}" apply -f "${ROOT_DIR}/examples/multi-cluster/mtls/peer-certificate.yaml"
  kubectl --context "${ctx}" apply -f "${ROOT_DIR}/examples/multi-cluster/mtls/peer-authentication.yaml"
done

echo "==> Applying Cluster A manifests"
kubectl --context kind-cluster-a create namespace stellar-nodes 2>/dev/null || true
kubectl --context kind-cluster-a apply -f "${ROOT_DIR}/examples/multi-cluster/cluster-a/"

echo "==> Applying Cluster B manifests"
kubectl --context kind-cluster-b create namespace stellar-nodes 2>/dev/null || true
kubectl --context kind-cluster-b apply -f "${ROOT_DIR}/examples/multi-cluster/cluster-b/"

echo "==> Verifying cross-cluster peer connectivity"
kubectl --context kind-cluster-a wait --for=condition=Ready \
  stellarnode validator-primary -n stellar-nodes --timeout=300s
kubectl --context kind-cluster-b wait --for=condition=Ready \
  stellarnode validator-standby -n stellar-nodes --timeout=300s

echo "==> Multi-cluster deployment complete."
echo "    Cluster A: kubectl --context kind-cluster-a get stellarnode -n stellar-nodes"
echo "    Cluster B: kubectl --context kind-cluster-b get stellarnode -n stellar-nodes"
