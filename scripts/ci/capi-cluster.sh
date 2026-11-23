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
#
# Thin Cluster-API / Kind wrapper so adding a Kubernetes version is a one-line
# matrix entry in .github/workflows/k8s-compat-matrix-advanced.yml.

set -euo pipefail

ACTION="${1:-create}"
K8S_VERSION="${2:-1.32.0}"
CLUSTER_NAME="${3:-stellar-capi-${K8S_VERSION//./-}}"
CAPI_VERSION="${CLUSTER_API_VERSION:-v1beta1}"

case "${ACTION}" in
  create)
    echo "Provisioning ephemeral cluster '${CLUSTER_NAME}' (K8s v${K8S_VERSION}, CAPI ${CAPI_VERSION})..."
    if command -v clusterctl >/dev/null 2>&1 && kubectl cluster-info >/dev/null 2>&1; then
      clusterctl generate cluster "${CLUSTER_NAME}" \
        --kubernetes-version "v${K8S_VERSION}" \
        --control-plane-machine-count=1 \
        --worker-machine-count=2 | kubectl apply -f -
    else
      cat <<EOF | kind create cluster --name "${CLUSTER_NAME}" --config=-
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    image: kindest/node:v${K8S_VERSION}
  - role: worker
    image: kindest/node:v${K8S_VERSION}
  - role: worker
    image: kindest/node:v${K8S_VERSION}
EOF
    fi
    ;;
  delete)
    echo "Tearing down ephemeral cluster '${CLUSTER_NAME}'..."
    if command -v clusterctl >/dev/null 2>&1 && kubectl get cluster "${CLUSTER_NAME}" >/dev/null 2>&1; then
      kubectl delete cluster "${CLUSTER_NAME}" --wait=false || true
    else
      kind delete cluster --name "${CLUSTER_NAME}" || true
    fi
    ;;
  *)
    echo "Usage: $0 {create|delete} <k8s_version> [cluster_name]" >&2
    exit 1
    ;;
esac
