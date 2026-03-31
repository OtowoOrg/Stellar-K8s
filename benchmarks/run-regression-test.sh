#!/bin/bash
#
# Local Performance Regression Testing Script
#
# This script replicates the GitHub Actions workflow locally for testing.
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLUSTER_NAME="benchmark"
NAMESPACE="stellar-benchmark"
BASELINE_VERSION="${BASELINE_VERSION:-v0.1.0}"
THRESHOLD="${THRESHOLD:-10}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

check_dependencies() {
    log_info "Checking dependencies..."

    local missing=()

    command -v kind >/dev/null 2>&1 || missing+=("kind")
    command -v kubectl >/dev/null 2>&1 || missing+=("kubectl")
    command -v k6 >/dev/null 2>&1 || missing+=("k6")
    command -v docker >/dev/null 2>&1 || missing+=("docker")
    command -v jq >/dev/null 2>&1 || missing+=("jq")
    command -v python3 >/dev/null 2>&1 || missing+=("python3")

    if [ ${#missing[@]} -gt 0 ]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_info "Install them with:"
        for dep in "${missing[@]}"; do
            case $dep in
                kind) echo "  curl -Lo ./kind https://kind.sigs.k8s.io/dl/v0.20.0/kind-linux-amd64 && chmod +x ./kind && sudo mv ./kind /usr/local/bin/" ;;
                kubectl) echo "  curl -LO https://dl.k8s.io/release/v1.28.0/bin/linux/amd64/kubectl && chmod +x kubectl && sudo mv kubectl /usr/local/bin/" ;;
                k6) echo "  sudo apt-get install k6  # or brew install k6 on macOS" ;;
                jq) echo "  sudo apt-get install jq  # or brew install jq on macOS" ;;
                python3) echo "  sudo apt-get install python3" ;;
            esac
        done
        exit 1
    fi

    log_info "✅ All dependencies installed"
}

setup_cluster() {
    log_info "Setting up kind cluster..."

    # Check if cluster already exists
    if kind get clusters | grep -q "^${CLUSTER_NAME}$"; then
        log_warn "Cluster '${CLUSTER_NAME}' already exists. Delete it? (y/n)"
        read -r response
        if [[ "$response" == "y" ]]; then
            kind delete cluster --name "$CLUSTER_NAME"
        else
            log_info "Using existing cluster"
            return 0
        fi
    fi

    # Create kind cluster
    cat > /tmp/kind-config.yaml <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    extraPortMappings:
      - containerPort: 30080
        hostPort: 30080
        protocol: TCP
  - role: worker
  - role: worker
EOF

    kind create cluster --name "$CLUSTER_NAME" --config /tmp/kind-config.yaml --wait 5m

    log_info "✅ Kind cluster created"
}

build_and_deploy() {
    log_info "Building operator..."

    cd "$PROJECT_ROOT"
    cargo build --release

    log_info "Building Docker image..."
    docker build -t stellar-operator:local .

    log_info "Loading image into kind..."
    kind load docker-image stellar-operator:local --name "$CLUSTER_NAME"

    log_info "Installing CRDs..."
    kubectl apply -f config/crd/stellarnode-crd.yaml
    kubectl wait --for condition=established --timeout=60s crd/stellarnodes.stellar.org

    log_info "Creating operator namespace..."
    kubectl create namespace stellar-system || true

    log_info "Deploying operator..."
    cat > /tmp/operator-deployment.yaml <<EOF
apiVersion: v1
kind: ServiceAccount
metadata:
  name: stellar-operator
  namespace: stellar-system
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: stellar-operator
rules:
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes", "stellarnodes/status"]
    verbs: ["*"]
  - apiGroups: [""]
    resources: ["pods", "services", "configmaps", "secrets", "events"]
    verbs: ["*"]
  - apiGroups: ["apps"]
    resources: ["statefulsets", "deployments"]
    verbs: ["*"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: stellar-operator
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: stellar-operator
subjects:
  - kind: ServiceAccount
    name: stellar-operator
    namespace: stellar-system
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: stellar-operator
  namespace: stellar-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: stellar-operator
  template:
    metadata:
      labels:
        app: stellar-operator
    spec:
      serviceAccountName: stellar-operator
      containers:
        - name: operator
          image: stellar-operator:local
          imagePullPolicy: Never
          ports:
            - containerPort: 8080
              name: http
            - containerPort: 9090
              name: metrics
          env:
            - name: RUST_LOG
              value: info
          resources:
            requests:
              cpu: 500m
              memory: 512Mi
            limits:
              cpu: 2000m
              memory: 2Gi
---
apiVersion: v1
kind: Service
metadata:
  name: stellar-operator
  namespace: stellar-system
spec:
  type: NodePort
  selector:
    app: stellar-operator
  ports:
    - name: http
      port: 8080
      targetPort: 8080
      nodePort: 30080
    - name: metrics
      port: 9090
      targetPort: 9090
EOF

    kubectl apply -f /tmp/operator-deployment.yaml

    log_info "Waiting for operator to be ready..."
    kubectl wait --for=condition=available deployment/stellar-operator \
        -n stellar-system --timeout=180s

    sleep 10

    log_info "✅ Operator deployed and ready"
}

run_benchmarks() {
    log_info "Running performance benchmarks..."

    mkdir -p "$PROJECT_ROOT/results"

    # Setup port forwarding
    log_info "Setting up port forwarding..."
    kubectl port-forward -n stellar-system svc/stellar-operator 8080:8080 &
    PF_PID=$!

    kubectl proxy --port=8001 &
    PROXY_PID=$!

    sleep 5

    # Verify connectivity
    if ! curl -sf http://localhost:8080/healthz > /dev/null; then
        log_error "Cannot reach operator health endpoint"
        kill $PF_PID $PROXY_PID 2>/dev/null || true
        exit 1
    fi

    log_info "Running k6 benchmarks..."

    BASELINE_FILE="$PROJECT_ROOT/benchmarks/baselines/${BASELINE_VERSION}.json"

    k6 run \
        --env BASE_URL=http://localhost:8080 \
        --env K8S_API_URL=http://localhost:8001 \
        --env NAMESPACE=$NAMESPACE \
        --env RUN_ID="local-$(date +%s)" \
        --env VERSION="local" \
        --env GIT_SHA="$(git rev-parse HEAD)" \
        --env BASELINE_FILE="$BASELINE_FILE" \
        --out json="$PROJECT_ROOT/results/operator-benchmark-raw.json" \
        "$PROJECT_ROOT/benchmarks/k6/operator-load-test.js" || true

    # Cleanup port forwarding
    kill $PF_PID $PROXY_PID 2>/dev/null || true

    log_info "✅ Benchmarks completed"
}

analyze_results() {
    log_info "Analyzing results..."

    BASELINE_FILE="$PROJECT_ROOT/benchmarks/baselines/${BASELINE_VERSION}.json"
    CURRENT_FILE="$PROJECT_ROOT/results/benchmark-summary.json"
    REPORT_FILE="$PROJECT_ROOT/results/regression-report.json"

    if [[ ! -f "$CURRENT_FILE" ]]; then
        log_error "Benchmark results not found: $CURRENT_FILE"
        exit 1
    fi

    if [[ ! -f "$BASELINE_FILE" ]]; then
        log_warn "Baseline not found: $BASELINE_FILE"
        log_info "Skipping regression analysis"
        return 0
    fi

    python3 "$PROJECT_ROOT/benchmarks/scripts/compare_benchmarks.py" compare \
        --current "$CURRENT_FILE" \
        --baseline "$BASELINE_FILE" \
        --threshold "$THRESHOLD" \
        --output "$REPORT_FILE" \
        --verbose

    # Check if regression was detected
    if [[ -f "$REPORT_FILE" ]]; then
        OVERALL_PASSED=$(jq -r '.overall_passed' "$REPORT_FILE")
        if [[ "$OVERALL_PASSED" == "false" ]]; then
            log_error "Performance regression detected!"
            return 1
        else
            log_info "✅ No regression detected"
        fi
    fi
}

cleanup() {
    log_info "Cleaning up..."

    # Kill port forwarding processes
    pkill -f "kubectl port-forward" || true
    pkill -f "kubectl proxy" || true

    # Delete kind cluster
    if kind get clusters | grep -q "^${CLUSTER_NAME}$"; then
        log_info "Deleting kind cluster..."
        kind delete cluster --name "$CLUSTER_NAME"
    fi

    log_info "✅ Cleanup completed"
}

show_results() {
    log_info "Performance Test Results:"
    echo ""

    if [[ -f "$PROJECT_ROOT/results/benchmark-summary.json" ]]; then
        cat "$PROJECT_ROOT/results/benchmark-summary.json" | jq '.'
    else
        log_warn "No results found"
    fi

    echo ""

    if [[ -f "$PROJECT_ROOT/results/regression-report.json" ]]; then
        log_info "Regression Report:"
        cat "$PROJECT_ROOT/results/regression-report.json" | jq '.'
    fi
}

usage() {
    cat <<EOF
Usage: $0 [COMMAND]

Commands:
    setup       Setup kind cluster and deploy operator
    run         Run performance benchmarks
    analyze     Analyze results and detect regressions
    cleanup     Delete kind cluster and cleanup
    full        Run full test (setup + run + analyze)
    show        Show results from last run

Environment Variables:
    BASELINE_VERSION    Baseline version to compare (default: v0.1.0)
    THRESHOLD           Regression threshold % (default: 10)

Examples:
    $0 full
    BASELINE_VERSION=v0.2.0 THRESHOLD=15 $0 full
    $0 setup && $0 run && $0 analyze
EOF
}

# Main
case "${1:-}" in
    setup)
        check_dependencies
        setup_cluster
        build_and_deploy
        ;;
    run)
        check_dependencies
        run_benchmarks
        ;;
    analyze)
        analyze_results
        ;;
    cleanup)
        cleanup
        ;;
    full)
        check_dependencies
        trap cleanup EXIT
        setup_cluster
        build_and_deploy
        run_benchmarks
        analyze_results
        show_results
        ;;
    show)
        show_results
        ;;
    *)
        usage
        exit 1
        ;;
esac
