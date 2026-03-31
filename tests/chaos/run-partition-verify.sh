#!/usr/bin/env bash
# Triggers Chaos Mesh network partition and verifies operator + StellarNode recovery.
# Usage: OPERATOR_NAMESPACE=stellar-system CHAOS_NAMESPACE=chaos-testing ./run-partition-verify.sh
set -euo pipefail

OP_NS="${OPERATOR_NAMESPACE:-stellar-system}"
CH_NS="${CHAOS_NAMESPACE:-chaos-testing}"
PARTITION_MANIFEST="$(cd "$(dirname "$0")" && pwd)/04-validator-peer-partition.yaml"

echo "== Stellar-K8s partition recovery check =="
echo "Applying NetworkChaos from ${PARTITION_MANIFEST} ..."
kubectl apply -f "${PARTITION_MANIFEST}" --namespace="${CH_NS}"

echo "Waiting 100s for partition + partial disruption ..."
sleep 100

echo "Removing experiment ..."
kubectl delete -f "${PARTITION_MANIFEST}" --namespace="${CH_NS}" --ignore-not-found

echo "Waiting for StellarNodes to report Ready (up to 300s) ..."
for i in $(seq 1 30); do
  NOT_READY=$(kubectl get stellarnode -n "${OP_NS}" --no-headers 2>/dev/null | grep -v True | wc -l | tr -d ' ')
  if [[ "${NOT_READY}" == "0" ]]; then
    echo "All StellarNodes Ready."
    kubectl get stellarnode -n "${OP_NS}"
    exit 0
  fi
  echo "  attempt ${i}: not all Ready yet ..."
  sleep 10
done

echo "Timeout: StellarNodes not satisfied Ready condition." >&2
kubectl get stellarnode -n "${OP_NS}" -o wide || true
exit 1
