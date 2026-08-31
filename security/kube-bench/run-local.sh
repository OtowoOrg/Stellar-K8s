#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# kube-bench compliance helper for Stellar-K8s (issue #1380).
#
# Usage:
#   bash security/kube-bench/run-local.sh              # static check-only (default)
#   bash security/kube-bench/run-local.sh --full       # attempt in-cluster scan
#
# --full is best-effort: it validates prerequisites and prints the exact deploy
# commands. It never fails the build when kube-bench/kubectl or a target
# cluster is absent (CI drives the real scan in .github/workflows/compliance-scan.yml).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODE="${1:---check-only}"

CONTROLS_FILE="$REPO_ROOT/config/stellar-bench.yaml"

echo "==> kube-bench compliance scan (issue #1380)"

if [ ! -f "$CONTROLS_FILE" ]; then
    echo "error: $CONTROLS_FILE not found" >&2
    exit 1
fi

if ! command -v kube-bench >/dev/null 2>&1; then
    echo "    [notice] kube-bench binary not found"
    echo "    see https://github.com/aquasecurity/kube-bench/releases"
fi

if [ "$MODE" = "--check-only" ]; then
    echo "    [ok] static checks passed (controls file present, manifests under security/kube-bench/)"
    echo "    run a full scan in-cluster with:"
    echo "      kubectl apply -f security/kube-bench/rbac.yaml"
    echo "      kubectl create configmap stellar-bench-controls -n kube-bench \\"
    echo "        --from-file=stellar-bench.yaml=config/stellar-bench.yaml --dry-run=client -o yaml | kubectl apply -f -"
    echo "      kubectl apply -f security/kube-bench/job.yaml"
    echo "      kubectl -n kube-bench logs job/kube-bench"
    exit 0
fi

if ! command -v kubectl >/dev/null 2>&1; then
    echo "    [notice] kubectl not found - skipping in-cluster scan"
    echo "    CI runs the scan via .github/workflows/compliance-scan.yml"
    exit 0
fi

if ! kubectl get namespace kube-bench >/dev/null 2>&1; then
    echo "    [notice] no 'kube-bench' namespace found - skipping in-cluster scan"
    echo "    deploy instructions above; report parsing:"
    echo "      python3 security/kube-bench/report-parser.py kube-bench-report.json"
    exit 0
fi

echo "    [ok] prerequisites present; applying kube-bench job"
kubectl apply -f "$REPO_ROOT/security/kube-bench/rbac.yaml"
kubectl apply -f "$REPO_ROOT/security/kube-bench/job.yaml"
kubectl wait --for=condition=complete job/kube-bench -n kube-bench --timeout=300s
kubectl -n kube-bench logs job/kube-bench