#!/usr/bin/env bash
# tests/chaos/run-chaos-tests.sh
#
# Runs the full Stellar-K8s chaos test suite against a local kind cluster.
#
# Prerequisites: Docker. Everything else is installed automatically.
#
# Usage:
#   chmod +x tests/chaos/run-chaos-tests.sh
#   ./tests/chaos/run-chaos-tests.sh
#
# To skip cluster setup (if you already have one running):
#   SKIP_SETUP=true ./tests/chaos/run-chaos-tests.sh

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[PASS]${NC}  $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
fail()    { echo -e "${RED}[FAIL]${NC}  $*"; exit 1; }

CLUSTER_NAME="${CLUSTER_NAME:-stellar-chaos}"
OPERATOR_NAMESPACE="${OPERATOR_NAMESPACE:-stellar-system}"
CHAOS_NAMESPACE="${CHAOS_NAMESPACE:-chaos-testing}"
SKIP_SETUP="${SKIP_SETUP:-false}"
CHAOS_MESH_VERSION="${CHAOS_MESH_VERSION:-2.6.3}"
RESULTS_DIR="tests/chaos/results/$(date +%Y%m%d-%H%M%S)"
OS="$(uname -s)"

mkdir -p "$RESULTS_DIR"

# ---- Helper: wait for pods to be Ready ----------------------------------------
wait_for_pods() {
  local namespace="$1" label="$2" timeout="${3:-120}"
  info "Waiting for pods ($label) in $namespace to be Ready (timeout: ${timeout}s)..."
  kubectl wait pod \
    --for=condition=Ready \
    --selector="$label" \
    --namespace="$namespace" \
    --timeout="${timeout}s" \
    && success "Pods ($label) are Ready" \
    || fail "Pods ($label) never became Ready within ${timeout}s"
}

# ---- Helper: assert all StellarNodes are Ready --------------------------------
assert_stellar_nodes_healthy() {
  local timeout="${1:-300}"
  local interval=10
  local elapsed=0

  info "Waiting for all StellarNodes to reach Ready condition (timeout: ${timeout}s)..."

  while [ $elapsed -lt $timeout ]; do
    local total not_ready
    total=$(kubectl get stellarnode --all-namespaces --no-headers 2>/dev/null | wc -l | tr -d ' ' || echo 0)
    not_ready=$(kubectl get stellarnode --all-namespaces -o json 2>/dev/null \
      | python3 -c "
import json,sys
data=json.load(sys.stdin)
count=sum(1 for i in data['items']
  if not any(c.get('type')=='Ready' and c.get('status')=='True'
             for c in i.get('status',{}).get('conditions',[])))
print(count)" 2>/dev/null || echo "$total")

    if [ "$total" -gt 0 ] && [ "$not_ready" -eq 0 ]; then
      success "All $total StellarNode(s) are Ready"
      return 0
    fi

    info "  ${elapsed}s — ${not_ready}/${total} nodes not yet Ready, retrying in ${interval}s..."
    sleep $interval
    elapsed=$((elapsed + interval))
  done

  fail "StellarNodes did not converge to Ready within ${timeout}s"
}

# ---- Helper: run one chaos experiment and verify recovery ---------------------
run_experiment() {
  local name="$1"
  local file="$2"
  local duration_seconds="$3"
  local recovery_timeout="$4"

  echo ""
  info "Running experiment: $name"
  info "---------------------------------------------------------"

  kubectl apply -f "$file" --namespace "$CHAOS_NAMESPACE"
  info "Chaos running for ${duration_seconds}s..."
  sleep "$duration_seconds"

  kubectl delete -f "$file" --namespace "$CHAOS_NAMESPACE" --ignore-not-found
  info "Chaos stopped. Waiting for operator to recover..."
  sleep 10

  wait_for_pods "$OPERATOR_NAMESPACE" "app=stellar-operator" 120
  assert_stellar_nodes_healthy "$recovery_timeout"

  kubectl logs \
    --selector=app=stellar-operator \
    --namespace="$OPERATOR_NAMESPACE" \
    --tail=200 \
    > "${RESULTS_DIR}/${name}-operator-logs.txt" 2>&1 || true

  success "Experiment '$name' PASSED"
}

# ---- Step 1: Install prerequisites -------------------------------------------
install_prerequisites() {
  info "Checking prerequisites..."

  if ! command -v kind &>/dev/null; then
    info "Installing kind..."
    if [ "$OS" = "Darwin" ]; then
      command -v brew &>/dev/null || fail "Homebrew not found. Install it from https://brew.sh then re-run."
      brew install kind
    else
      mkdir -p "$HOME/.local/bin"
      curl -Lo "$HOME/.local/bin/kind" \
        "https://kind.sigs.k8s.io/dl/v0.24.0/kind-linux-amd64"
      chmod +x "$HOME/.local/bin/kind"
      export PATH="$HOME/.local/bin:$PATH"
    fi
    success "kind installed"
  else
    success "kind already installed"
  fi

  if ! command -v kubectl &>/dev/null; then
    info "Installing kubectl..."
    if [ "$OS" = "Darwin" ]; then
      brew install kubectl
    else
      mkdir -p "$HOME/.local/bin"
      curl -Lo "$HOME/.local/bin/kubectl" \
        "https://dl.k8s.io/release/$(curl -sL https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
      chmod +x "$HOME/.local/bin/kubectl"
      export PATH="$HOME/.local/bin:$PATH"
    fi
    success "kubectl installed"
  else
    success "kubectl already installed"
  fi

  if ! command -v helm &>/dev/null; then
    info "Installing helm..."
    if [ "$OS" = "Darwin" ]; then
      brew install helm
    else
      curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
    fi
    success "helm installed"
  else
    success "helm already installed"
  fi
}

# ---- Step 2: Create kind cluster ---------------------------------------------
setup_cluster() {
  if kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
    warn "Cluster '$CLUSTER_NAME' already exists, reusing it."
    kind export kubeconfig --name "$CLUSTER_NAME"
    return
  fi

  info "Creating kind cluster '$CLUSTER_NAME'..."
  kind create cluster --name "$CLUSTER_NAME" --config - <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
  - role: worker
  - role: worker
EOF
  success "Cluster '$CLUSTER_NAME' created"
}

# ---- Step 3: Install Chaos Mesh ----------------------------------------------
install_chaos_mesh() {
  if kubectl get namespace chaos-mesh &>/dev/null; then
    warn "Chaos Mesh already installed, skipping."
    return
  fi

  info "Installing Chaos Mesh v${CHAOS_MESH_VERSION}..."
  helm repo add chaos-mesh https://charts.chaos-mesh.org
  helm repo update

  kubectl create namespace chaos-mesh
  helm install chaos-mesh chaos-mesh/chaos-mesh \
    --namespace chaos-mesh \
    --version "$CHAOS_MESH_VERSION" \
    --set chaosDaemon.runtime=containerd \
    --set chaosDaemon.socketPath=/run/containerd/containerd.sock \
    --wait --timeout=5m

  success "Chaos Mesh installed"

  kubectl create namespace "$CHAOS_NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

  kubectl apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: chaos-mesh-stellar-system
  namespace: $OPERATOR_NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: chaos-mesh-chaos-controller-manager-target-namespace
subjects:
  - kind: ServiceAccount
    name: chaos-controller-manager
    namespace: chaos-mesh
EOF
}

# ---- Step 4: Install the operator and a test StellarNode ---------------------
install_operator() {
  info "Installing StellarNode CRD..."
  kubectl apply -f config/crd/stellarnode-crd.yaml
  kubectl create namespace "$OPERATOR_NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

  info "Building operator image..."
  docker build -t stellar-operator:chaos-test .
  kind load docker-image stellar-operator:chaos-test --name "$CLUSTER_NAME"

  info "Deploying operator..."
  kubectl apply -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: stellar-operator
  namespace: $OPERATOR_NAMESPACE
  labels:
    app: stellar-operator
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
      containers:
        - name: operator
          image: stellar-operator:chaos-test
          imagePullPolicy: Never
          command: ["stellar-operator", "run"]
          env:
            - name: RUST_LOG
              value: info
            - name: OPERATOR_NAMESPACE
              value: $OPERATOR_NAMESPACE
          resources:
            requests:
              cpu: 200m
              memory: 256Mi
            limits:
              cpu: 1000m
              memory: 512Mi
EOF

  wait_for_pods "$OPERATOR_NAMESPACE" "app=stellar-operator" 120

  info "Creating test StellarNode..."
  kubectl apply -f - <<EOF
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: chaos-test-horizon
  namespace: $OPERATOR_NAMESPACE
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

  info "Waiting 30s for initial reconciliation..."
  sleep 30
}

# ---- Main --------------------------------------------------------------------
main() {
  echo ""
  echo "=================================================="
  echo "  Stellar-K8s Chaos Engineering Test Suite"
  echo "=================================================="
  echo ""

  install_prerequisites

  if [ "$SKIP_SETUP" != "true" ]; then
    setup_cluster
    install_chaos_mesh
    install_operator
  else
    warn "SKIP_SETUP=true — skipping cluster setup"
    kind export kubeconfig --name "$CLUSTER_NAME" 2>/dev/null || true
  fi

  OVERALL_PASS=true

  # Experiment 1: pod kill — wait 40s, recovery timeout 180s
  run_experiment "01-operator-pod-kill" "tests/chaos/01-operator-pod-kill.yaml" 40 180 || OVERALL_PASS=false

  # Experiment 2: network partition — wait 70s, recovery timeout 300s
  run_experiment "02-network-partition" "tests/chaos/02-network-partition.yaml" 70 300 || OVERALL_PASS=false

  # Experiment 3: API latency — wait 130s, recovery timeout 600s
  run_experiment "03-api-latency" "tests/chaos/03-api-latency.yaml" 130 600 || OVERALL_PASS=false

  echo ""
  echo "=================================================="
  echo "  Results saved to: $RESULTS_DIR"
  echo "=================================================="
  echo ""

  if [ "$OVERALL_PASS" = "true" ]; then
    success "ALL EXPERIMENTS PASSED"
    exit 0
  else
    fail "ONE OR MORE EXPERIMENTS FAILED — check logs in $RESULTS_DIR"
  fi
}

main "$@"