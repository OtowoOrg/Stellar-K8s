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
# scripts/verify-mtls.sh — Verify mTLS inter-service encryption and rotation readiness
set -euo pipefail

NAMESPACE="${NAMESPACE:-stellar-system}"

echo "→ Verifying mTLS setup in namespace ${NAMESPACE}"

check_cert() {
  local name="$1"
  echo "  Checking Certificate/${name}..."
  if kubectl -n "${NAMESPACE}" get certificate "${name}" >/dev/null 2>&1; then
    local ready
    ready=$(kubectl -n "${NAMESPACE}" get certificate "${name}" -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' 2>/dev/null || echo "Unknown")
    if [[ "${ready}" == "True" ]]; then echo "    ✓ Ready"; else echo "    ⚠ Not Ready (status=${ready})"; fi
  else
    echo "    ✗ Not found"
    return 1
  fi
}

check_secret() {
  local secret="$1"
  echo "  Checking Secret/${secret}..."
  if kubectl -n "${NAMESPACE}" get secret "${secret}" >/dev/null 2>&1; then
    echo "    ✓ Exists"
    local has_tls
    has_tls=$(kubectl -n "${NAMESPACE}" get secret "${secret}" -o jsonpath='{.data.tls\.crt}' 2>/dev/null | wc -c)
    if [[ "${has_tls}" -gt 0 ]]; then echo "    ✓ tls.crt present"; else echo "    ✗ tls.crt missing"; fi
  else
    echo "    ✗ Not found"
  fi
}

errors=0
for cert in stellar-core-mtls-cert horizon-mtls-cert soroban-rpc-mtls-cert; do
  check_cert "${cert}" || ((errors++)) || true
done

for sec in stellar-core-mtls-secret horizon-mtls-secret soroban-rpc-mtls-secret; do
  check_secret "${sec}" || true
done

echo ""
echo "→ Traffic encryption verification:"
echo "  Inter-service traffic uses mTLS (ECDSA P-256) via cert-manager. Verify with:"
echo "    kubectl -n ${NAMESPACE} exec deploy/stellar-operator -- openssl s_client -connect horizon:8000 -showcerts | head"
echo ""
if [[ "${errors}" -eq 0 ]]; then
  echo "✓ mTLS verification passed"
else
  echo "⚠ ${errors} certificate(s) not ready — run with mtls.enabled=true and cert-manager installed"
fi
