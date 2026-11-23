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
# Runs Stellar-K8s CRD and operator conformance checks against a target K8s cluster.

set -euo pipefail

K8S_VERSION="${1:-1.32}"
NAMESPACE="${OPERATOR_NAMESPACE:-stellar-system}"

echo "=== Running Stellar-K8s conformance suite on Kubernetes ${K8S_VERSION} ==="

for crd in config/crd/*.yaml; do
  if [[ -f "${crd}" ]]; then
    echo "Applying CRD: ${crd}"
    kubectl apply -f "${crd}"
  fi
done

echo "Verifying CRDs are Established..."
for crd_name in $(kubectl get crds -o jsonpath='{.items[*].metadata.name}'); do
  if [[ "${crd_name}" == *stellar* || "${crd_name}" == *backupplan* ]]; then
    kubectl wait --for=condition=Established "crd/${crd_name}" --timeout=60s
  fi
done

echo "Verifying operator deployment readiness in ${NAMESPACE}..."
if kubectl get deployment -n "${NAMESPACE}" stellar-operator >/dev/null 2>&1; then
  kubectl rollout status deployment/stellar-operator -n "${NAMESPACE}" --timeout=120s
fi

echo "Running sample manifest dry-run validation..."
for sample in config/samples/*.yaml; do
  [[ "$(basename "${sample}")" == invalid-* ]] && continue
  kubectl apply --dry-run=server -f "${sample}" >/dev/null
done

echo "✅ Conformance suite passed for Kubernetes ${K8S_VERSION}"
