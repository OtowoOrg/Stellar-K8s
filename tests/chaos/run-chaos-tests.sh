#!/usr/bin/env bash
# tests/chaos/run-chaos-tests.sh
#
# End-to-end chaos engineering runner for Stellar-K8s.
# Runs all 10 experiments against a local kind cluster and generates
# a resilience report.
#
# Usage:
#   ./tests/chaos/run-chaos-tests.sh                  # full run
#   SKIP_SETUP=true ./tests/chaos/run-chaos-tests.sh  # skip cluster creation
#   EXPERIMENTS="01 02 05" ./tests/chaos/run-chaos-tests.sh  # run subset
#   SKIP_EXPERIMENTS="09" ./tests/chaos/run-chaos-tests.sh   # skip cascading
#
# Environment variables:
#   SKIP_SETUP          Skip kind cluster creation (default: false)
#   EXPERIMENTS         Space-separated list of experiment IDs to run (default: all)
#   SKIP_EXPERIMENTS    Space-separated list of experiment IDs to skip
#   CLUSTER_NAME        kind cluster name (default: stellar-chaos)
#   OPERATOR_NS         Operator namespace (default: stellar-system)
#   CHAOS_NS            Chaos Mesh namespace (default: chaos-testing)
#   CHAOS_MESH_VERSION  Chaos Mesh Helm chart version (default: 2.6.3)
#   IMAGE_TAG           Docker image tag (default: chaos-test)
#   RECOVERY_TIMEOUT    Max seconds to wait for recovery (default: 300)

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────────────────
CLUSTER_NAME="${CLUSTER_NAME:-stellar-chaos}"
OPERATOR_NS="${OPERATOR_NS:-stellar-system}"
CHAOS_NS="${CHAOS_NS:-chaos-testing}"
CHAOS_MESH_VERSION="${CHAOS_MESH_VERSION:-2.6.3}"
IMAGE_TAG="${IMAGE_TAG:-chaos-test}"
RECOVERY_TIMEOUT="${RECOVERY_TIMEOUT:-300}"
SKIP_SETUP="${SKIP_SETUP:-false}"
ALL_EXPERIMENTS="01 02 03 04 05 06 07 08 09 10"
EXPERIMENTS="${EXPERIMENTS:-$ALL_EXPERIMENTS}"
SKIP_EXPERIMENTS="${SKIP_EXPERIMENTS:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUN_ID="$(date +%Y%m%d-%H%M%S)"
RESULTS_DIR="$SCRIPT_DIR/results/$RUN_ID"

# ── Colours ────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; BOLD='\033[1m'; RESET='\033[0m'

log()  { echo -e "${BLUE}[chaos]${RESET} $*"; }
ok()   { echo -e "${GREEN}[✓]${RESET} $*"; }
warn() { echo -e "${YELLOW}[⚠]${RESET} $*"; }
fail() { echo -e "${RED}[✗]${RESET} $*"; }
sep()  { echo -e "${BOLD}────────────────────────────────────────────────${RESET}"; }

# ── Dependency checks ──────────────────────────────────────────────────────
check_deps() {
    local missing=()
    for cmd in docker kubectl helm kind python3; do
        command -v "$cmd" &>/dev/null || missing+=("$cmd")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing dependencies: ${missing[*]}"
        echo "Install them and re-run, or set SKIP_SETUP=true if cluster already exists."
        exit 1
    fi
}

# ── Cluster setup ──────────────────────────────────────────────────────────
setup_cluster() {
    log "Creating kind cluster: $CLUSTER_NAME"
    kind create cluster --name "$CLUSTER_NAME" --config - <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
  - role: worker
  - role: worker
EOF

    log "Building operator Docker image: stellar-operator:$IMAGE_TAG"
    docker build -t "stellar-operator:$IMAGE_TAG" "$PROJECT_ROOT"
    kind load docker-image "stellar-operator:$IMAGE_TAG" --name "$CLUSTER_NAME"

    log "Installing operator into $OPERATOR_NS"
    kubectl create namespace "$OPERATOR_NS" --dry-run=client -o yaml | kubectl apply -f -
    helm upgrade --install stellar-operator "$PROJECT_ROOT/charts/stellar-operator" \
        --namespace "$OPERATOR_NS" \
        --set image.tag="$IMAGE_TAG" \
        --set image.pullPolicy=Never \
        --wait --timeout=5m

    log "Installing Chaos Mesh $CHAOS_MESH_VERSION"
    helm repo add chaos-mesh https://charts.chaos-mesh.org --force-update
    helm repo update
    kubectl create namespace chaos-mesh --dry-run=client -o yaml | kubectl apply -f -
    helm upgrade --install chaos-mesh chaos-mesh/chaos-mesh \
        --namespace chaos-mesh \
        --version "$CHAOS_MESH_VERSION" \
        --set chaosDaemon.runtime=containerd \
        --set chaosDaemon.socketPath=/run/containerd/containerd.sock \
        --wait --timeout=5m

    kubectl create namespace "$CHAOS_NS" --dry-run=client -o yaml | kubectl apply -f -

    # Grant Chaos Mesh permission to target stellar-system
    kubectl apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: chaos-mesh-stellar-system
  namespace: $OPERATOR_NS
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: chaos-mesh-chaos-controller-manager-target-namespace
subjects:
  - kind: ServiceAccount
    name: chaos-controller-manager
    namespace: chaos-mesh
EOF

    log "Deploying test StellarNode"
    kubectl apply -f - <<EOF
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: chaos-test-horizon
  namespace: $OPERATOR_NS
spec:
  nodeType: Horizon
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  horizonConfig:
    databaseSecretRef: horizon-db-secret
    enableIngest: false
    stellarCoreUrl: "http://localhost:11626"
    ingestWorkers: 1
    enableExperimentalIngestion: false
    autoMigration: false
EOF
    sleep 30
    ok "Cluster setup complete"
}

# ── Experiment runner ──────────────────────────────────────────────────────
run_experiment() {
    local exp_id="$1"
    local manifest="$SCRIPT_DIR/${exp_id}-"*.yaml
    local log_file="$RESULTS_DIR/exp${exp_id}-operator-logs.txt"
    local meta_file="$RESULTS_DIR/exp${exp_id}-meta.json"

    # Resolve glob
    manifest=$(ls "$SCRIPT_DIR/${exp_id}-"*.yaml 2>/dev/null | head -1 || true)
    if [[ -z "$manifest" ]]; then
        warn "No manifest found for experiment $exp_id — skipping"
        return 0
    fi

    sep
    log "Running Experiment $exp_id: $(basename "$manifest" .yaml)"

    # Determine duration from manifest
    local duration_secs
    duration_secs=$(grep -oP 'duration:\s*"\K[0-9]+(?=s")' "$manifest" | head -1 || echo "60")
    local wait_secs=$(( duration_secs + 30 ))

    local start_ts
    start_ts=$(date +%s)

    # Apply experiment
    kubectl apply -f "$manifest" --namespace="$CHAOS_NS"

    log "Experiment running for ${duration_secs}s (waiting ${wait_secs}s total)..."
    sleep "$wait_secs"

    # Remove experiment
    kubectl delete -f "$manifest" --namespace="$CHAOS_NS" --ignore-not-found

    # Collect operator logs
    kubectl logs \
        --selector=app=stellar-operator \
        --namespace="$OPERATOR_NS" \
        --tail=1000 \
        --since="${wait_secs}s" \
        > "$log_file" 2>&1 || true

    # Wait for recovery
    local recovery_start
    recovery_start=$(date +%s)
    local recovered=false

    log "Waiting for operator recovery (timeout: ${RECOVERY_TIMEOUT}s)..."
    if kubectl wait pod \
        --for=condition=Ready \
        --selector=app=stellar-operator \
        --namespace="$OPERATOR_NS" \
        --timeout="${RECOVERY_TIMEOUT}s" 2>/dev/null; then
        recovered=true
    fi

    local recovery_end
    recovery_end=$(date +%s)
    local recovery_secs=$(( recovery_end - recovery_start ))

    # Write metadata
    cat > "$meta_file" <<EOF
{
  "experiment_id": "$exp_id",
  "manifest": "$(basename "$manifest")",
  "start_timestamp": $start_ts,
  "duration_secs": $duration_secs,
  "recovery_time_secs": $recovery_secs,
  "recovered": $recovered
}
EOF

    if [[ "$recovered" == "true" ]]; then
        ok "Experiment $exp_id: operator recovered in ${recovery_secs}s"
    else
        fail "Experiment $exp_id: operator did NOT recover within ${RECOVERY_TIMEOUT}s"
    fi
}

# ── Should we run this experiment? ────────────────────────────────────────
should_run() {
    local exp_id="$1"
    # Check if in EXPERIMENTS list
    if [[ ! " $EXPERIMENTS " =~ " $exp_id " ]]; then
        return 1
    fi
    # Check if in SKIP_EXPERIMENTS list
    if [[ -n "$SKIP_EXPERIMENTS" && " $SKIP_EXPERIMENTS " =~ " $exp_id " ]]; then
        warn "Skipping experiment $exp_id (in SKIP_EXPERIMENTS)"
        return 1
    fi
    return 0
}

# ── Report generation ──────────────────────────────────────────────────────
generate_report() {
    log "Generating resilience report..."
    if command -v python3 &>/dev/null; then
        python3 "$SCRIPT_DIR/generate_report.py" \
            --results-dir "$RESULTS_DIR" \
            --output-format both \
            --run-id "$RUN_ID" || true
    else
        # Fallback to shell script
        bash "$SCRIPT_DIR/generate-report.sh" "$RESULTS_DIR" || true
    fi
}

# ── Main ───────────────────────────────────────────────────────────────────
main() {
    sep
    echo -e "${BOLD}🔥 Stellar-K8s Chaos Engineering Suite${RESET}"
    echo -e "   Run ID: $RUN_ID"
    echo -e "   Results: $RESULTS_DIR"
    sep

    check_deps
    mkdir -p "$RESULTS_DIR"

    if [[ "$SKIP_SETUP" != "true" ]]; then
        setup_cluster
    else
        log "Skipping cluster setup (SKIP_SETUP=true)"
    fi

    local pass_count=0
    local fail_count=0
    local skip_count=0

    for exp_id in $ALL_EXPERIMENTS; do
        if should_run "$exp_id"; then
            if run_experiment "$exp_id"; then
                (( pass_count++ )) || true
            else
                (( fail_count++ )) || true
            fi
        else
            (( skip_count++ )) || true
        fi
    done

    generate_report

    sep
    echo -e "${BOLD}Results Summary${RESET}"
    echo -e "  ✅ Passed:  $pass_count"
    echo -e "  ❌ Failed:  $fail_count"
    echo -e "  ⏭  Skipped: $skip_count"
    echo -e "  📄 Report:  $RESULTS_DIR/resilience-report.md"
    sep

    if [[ $fail_count -gt 0 ]]; then
        fail "Chaos suite FAILED — $fail_count experiment(s) did not recover"
        exit 1
    else
        ok "Chaos suite PASSED"
    fi
}

main "$@"
