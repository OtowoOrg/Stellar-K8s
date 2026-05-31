#!/usr/bin/env bash
# =============================================================================
# debug-network.sh — Stellar-K8s networking diagnostic script
#
# Runs a series of connectivity checks from within (or targeting) the cluster
# to help diagnose common issues: Connection Refused, No Route to Host, SCP
# handshake failures, DNS problems, and NetworkPolicy misconfigurations.
#
# Usage:
#   ./scripts/debug-network.sh [OPTIONS]
#
# Options:
#   -n, --namespace NS      Namespace to inspect (default: stellar-system)
#   -N, --node-name NAME    StellarNode name to target (optional)
#   -p, --peer IP:PORT      External peer to test P2P reachability (optional)
#   -t, --timeout SECS      Per-check timeout in seconds (default: 5)
#   -h, --help              Show this help message
#
# Examples:
#   # Check all validators in the stellar-nodes namespace
#   ./scripts/debug-network.sh -n stellar-nodes
#
#   # Check a specific node and test an external peer
#   ./scripts/debug-network.sh -n stellar-nodes -N my-validator -p 1.2.3.4:11625
#
# Requirements:
#   kubectl must be configured and pointing at the target cluster.
# =============================================================================

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
NAMESPACE="stellar-system"
NODE_NAME=""
EXTERNAL_PEER=""
TIMEOUT=5
DEBUG_IMAGE="nicolaka/netshoot"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

pass()  { echo -e "  ${GREEN}✔${RESET}  $*"; }
fail()  { echo -e "  ${RED}✘${RESET}  $*"; }
warn()  { echo -e "  ${YELLOW}⚠${RESET}  $*"; }
info()  { echo -e "  ${CYAN}ℹ${RESET}  $*"; }
header(){ echo -e "\n${BOLD}${CYAN}══ $* ══${RESET}"; }

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    -n|--namespace)   NAMESPACE="$2";     shift 2 ;;
    -N|--node-name)   NODE_NAME="$2";     shift 2 ;;
    -p|--peer)        EXTERNAL_PEER="$2"; shift 2 ;;
    -t|--timeout)     TIMEOUT="$2";       shift 2 ;;
    -h|--help)
      sed -n '/^# Usage/,/^# Requirements/p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# ── Helpers ───────────────────────────────────────────────────────────────────

# Run a command inside a temporary debug pod in the target namespace.
# Usage: run_in_pod <command...>
run_in_pod() {
  kubectl run -it --rm stellar-netdebug \
    --image="${DEBUG_IMAGE}" \
    --restart=Never \
    --namespace="${NAMESPACE}" \
    --timeout="${TIMEOUT}s" \
    --quiet \
    -- bash -c "$*" 2>/dev/null || true
}

# Check if a TCP port is open from inside the cluster.
# Usage: check_tcp <host> <port> <label>
check_tcp() {
  local host="$1" port="$2" label="$3"
  local result
  result=$(run_in_pod "nc -zw${TIMEOUT} ${host} ${port} && echo OK || echo FAIL")
  if echo "$result" | grep -q "^OK"; then
    pass "${label} (${host}:${port}) — reachable"
  else
    fail "${label} (${host}:${port}) — NOT reachable"
  fi
}

# ── Preflight ─────────────────────────────────────────────────────────────────
header "Preflight"

if ! command -v kubectl &>/dev/null; then
  fail "kubectl not found in PATH"
  exit 1
fi
pass "kubectl found: $(kubectl version --client --short 2>/dev/null | head -1)"

CONTEXT=$(kubectl config current-context 2>/dev/null || echo "unknown")
info "Cluster context: ${CONTEXT}"
info "Target namespace: ${NAMESPACE}"

# ── 1. Operator health ────────────────────────────────────────────────────────
header "1. Operator Health"

OPERATOR_POD=$(kubectl get pods -n stellar-system \
  -l app.kubernetes.io/name=stellar-operator \
  -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)

if [[ -n "${OPERATOR_POD}" ]]; then
  OPERATOR_STATUS=$(kubectl get pod "${OPERATOR_POD}" -n stellar-system \
    -o jsonpath='{.status.phase}' 2>/dev/null)
  if [[ "${OPERATOR_STATUS}" == "Running" ]]; then
    pass "Operator pod ${OPERATOR_POD} is Running"
  else
    fail "Operator pod ${OPERATOR_POD} is ${OPERATOR_STATUS}"
  fi
else
  warn "No operator pod found in stellar-system (may be in a different namespace)"
fi

# ── 2. StellarNode resources ──────────────────────────────────────────────────
header "2. StellarNode Resources (namespace: ${NAMESPACE})"

NODES=$(kubectl get stellarnodes -n "${NAMESPACE}" \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.spec.nodeType}{"\t"}{.status.phase}{"\n"}{end}' \
  2>/dev/null || true)

if [[ -z "${NODES}" ]]; then
  warn "No StellarNode resources found in namespace ${NAMESPACE}"
else
  echo ""
  printf "  %-30s %-15s %-15s\n" "NAME" "TYPE" "PHASE"
  printf "  %-30s %-15s %-15s\n" "----" "----" "-----"
  while IFS=$'\t' read -r name ntype phase; do
    printf "  %-30s %-15s %-15s\n" "${name}" "${ntype}" "${phase:-unknown}"
  done <<< "${NODES}"
fi

# ── 3. Pod status ─────────────────────────────────────────────────────────────
header "3. Pod Status (namespace: ${NAMESPACE})"

PODS=$(kubectl get pods -n "${NAMESPACE}" \
  -l app.kubernetes.io/name=stellar-node \
  -o wide 2>/dev/null || true)

if [[ -z "${PODS}" ]]; then
  warn "No stellar-node pods found in namespace ${NAMESPACE}"
else
  echo ""
  echo "${PODS}" | while IFS= read -r line; do
    echo "  ${line}"
  done
fi

# ── 4. Service endpoints ──────────────────────────────────────────────────────
header "4. Service Endpoints (namespace: ${NAMESPACE})"

SERVICES=$(kubectl get svc -n "${NAMESPACE}" \
  -l app.kubernetes.io/name=stellar-node \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.spec.type}{"\t"}{.spec.clusterIP}{"\t"}{.status.loadBalancer.ingress[0].ip}{"\n"}{end}' \
  2>/dev/null || true)

if [[ -z "${SERVICES}" ]]; then
  warn "No stellar-node Services found in namespace ${NAMESPACE}"
else
  echo ""
  printf "  %-35s %-15s %-18s %-18s\n" "SERVICE" "TYPE" "CLUSTER-IP" "EXTERNAL-IP"
  printf "  %-35s %-15s %-18s %-18s\n" "-------" "----" "----------" "-----------"
  while IFS=$'\t' read -r svc_name svc_type cluster_ip ext_ip; do
    printf "  %-35s %-15s %-18s %-18s\n" \
      "${svc_name}" "${svc_type}" "${cluster_ip}" "${ext_ip:-(none)}"
  done <<< "${SERVICES}"
fi

# Check for empty endpoints (selector mismatch)
echo ""
info "Checking for Services with no ready endpoints..."
kubectl get endpoints -n "${NAMESPACE}" \
  -l app.kubernetes.io/name=stellar-node \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.subsets}{"\n"}{end}' \
  2>/dev/null | while IFS=$'\t' read -r ep_name subsets; do
  if [[ -z "${subsets}" || "${subsets}" == "null" ]]; then
    fail "Endpoint ${ep_name} has NO ready addresses — selector may not match pods"
  else
    pass "Endpoint ${ep_name} has ready addresses"
  fi
done || warn "Could not check endpoints (no labeled services found)"

# ── 5. NetworkPolicies ────────────────────────────────────────────────────────
header "5. NetworkPolicies (namespace: ${NAMESPACE})"

NP_COUNT=$(kubectl get networkpolicies -n "${NAMESPACE}" \
  --no-headers 2>/dev/null | wc -l | tr -d ' ')

if [[ "${NP_COUNT}" -eq 0 ]]; then
  warn "No NetworkPolicies found — all traffic is allowed (no isolation)"
else
  info "Found ${NP_COUNT} NetworkPolicy/ies:"
  kubectl get networkpolicies -n "${NAMESPACE}" \
    -o jsonpath='{range .items[*]}  • {.metadata.name}{"\n"}{end}' 2>/dev/null

  # Check for default-deny without P2P allow
  DEFAULT_DENY=$(kubectl get networkpolicies -n "${NAMESPACE}" \
    -o jsonpath='{range .items[*]}{.spec.podSelector}{"\t"}{.spec.policyTypes}{"\n"}{end}' \
    2>/dev/null | grep -c '{}' || true)
  if [[ "${DEFAULT_DENY}" -gt 0 ]]; then
    warn "Detected a policy with empty podSelector (default-deny). Ensure port 11625 is explicitly allowed."
  fi
fi

# ── 6. DNS resolution ─────────────────────────────────────────────────────────
header "6. DNS Resolution"

info "Testing cluster DNS from a debug pod..."

# Test internal DNS
DNS_RESULT=$(run_in_pod \
  "nslookup kubernetes.default.svc.cluster.local 2>&1 | grep -c 'Address' || echo 0")
if [[ "${DNS_RESULT:-0}" -gt 0 ]]; then
  pass "Internal DNS (kubernetes.default.svc.cluster.local) resolves"
else
  fail "Internal DNS resolution failed — check CoreDNS"
fi

# Test external DNS
EXT_DNS=$(run_in_pod \
  "nslookup history.stellar.org 2>&1 | grep -c 'Address' || echo 0")
if [[ "${EXT_DNS:-0}" -gt 0 ]]; then
  pass "External DNS (history.stellar.org) resolves"
else
  fail "External DNS resolution failed — check egress NetworkPolicy (port 53 UDP/TCP)"
fi

# ── 7. P2P connectivity (internal) ───────────────────────────────────────────
header "7. Internal P2P Connectivity (port 11625)"

VALIDATOR_SVCS=$(kubectl get svc -n "${NAMESPACE}" \
  -l app.kubernetes.io/component=stellar-validator \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null || true)

if [[ -z "${VALIDATOR_SVCS}" ]]; then
  # Fall back to all stellar-node services
  VALIDATOR_SVCS=$(kubectl get svc -n "${NAMESPACE}" \
    -l app.kubernetes.io/name=stellar-node \
    -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null || true)
fi

if [[ -z "${VALIDATOR_SVCS}" ]]; then
  warn "No validator Services found — skipping internal P2P check"
else
  while IFS= read -r svc; do
    check_tcp "${svc}.${NAMESPACE}.svc.cluster.local" "11625" "P2P ${svc}"
  done <<< "${VALIDATOR_SVCS}"
fi

# ── 8. Admin HTTP port (11626) ────────────────────────────────────────────────
header "8. Stellar Core Admin Port (11626)"

if [[ -n "${NODE_NAME}" ]]; then
  POD_NAME=$(kubectl get pods -n "${NAMESPACE}" \
    -l "app.kubernetes.io/instance=${NODE_NAME}" \
    -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)

  if [[ -n "${POD_NAME}" ]]; then
    ADMIN_RESULT=$(kubectl exec -n "${NAMESPACE}" "${POD_NAME}" -- \
      curl -sf --max-time "${TIMEOUT}" http://localhost:11626/info 2>/dev/null | \
      python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('info',{}).get('state','unknown'))" \
      2>/dev/null || echo "unreachable")
    if [[ "${ADMIN_RESULT}" != "unreachable" ]]; then
      pass "Admin port 11626 on ${POD_NAME}: state=${ADMIN_RESULT}"
    else
      fail "Admin port 11626 on ${POD_NAME} is not responding"
    fi
  else
    warn "No pod found for StellarNode ${NODE_NAME}"
  fi
else
  info "Pass -N <node-name> to check the admin port on a specific node"
fi

# ── 9. External peer reachability ─────────────────────────────────────────────
header "9. External Peer Reachability"

if [[ -n "${EXTERNAL_PEER}" ]]; then
  EXT_HOST="${EXTERNAL_PEER%%:*}"
  EXT_PORT="${EXTERNAL_PEER##*:}"
  check_tcp "${EXT_HOST}" "${EXT_PORT}" "External peer"
else
  info "Pass -p <ip>:<port> to test an external peer (e.g. -p 1.2.3.4:11625)"
fi

# ── 10. History archive access ────────────────────────────────────────────────
header "10. History Archive Access (HTTPS)"

ARCHIVE_RESULT=$(run_in_pod \
  "curl -sf --max-time ${TIMEOUT} https://history.stellar.org/prd/core-live/core_live_001/.well-known/stellar-history.json \
   | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get(\"currentLedger\",\"unknown\"))' \
   2>/dev/null || echo FAIL")

if [[ "${ARCHIVE_RESULT}" != "FAIL" && -n "${ARCHIVE_RESULT}" ]]; then
  pass "Stellar history archive reachable (currentLedger=${ARCHIVE_RESULT})"
else
  fail "Cannot reach Stellar history archive — check egress NetworkPolicy (port 443 TCP)"
fi

# ── 11. TLS certificate check ─────────────────────────────────────────────────
header "11. mTLS Certificate Status (namespace: ${NAMESPACE})"

CERT_SECRETS=$(kubectl get secrets -n "${NAMESPACE}" \
  --field-selector type=kubernetes.io/tls \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null || true)

if [[ -z "${CERT_SECRETS}" ]]; then
  warn "No TLS secrets found in namespace ${NAMESPACE}"
else
  while IFS= read -r secret; do
    EXPIRY=$(kubectl get secret "${secret}" -n "${NAMESPACE}" \
      -o jsonpath='{.data.tls\.crt}' 2>/dev/null | \
      base64 -d 2>/dev/null | \
      openssl x509 -noout -enddate 2>/dev/null | \
      sed 's/notAfter=//' || echo "parse error")
    if [[ "${EXPIRY}" == "parse error" ]]; then
      warn "Secret ${secret}: could not parse certificate"
    else
      # Check if expired
      if openssl x509 -checkend 0 -noout \
           <(kubectl get secret "${secret}" -n "${NAMESPACE}" \
             -o jsonpath='{.data.tls\.crt}' 2>/dev/null | base64 -d 2>/dev/null) \
           2>/dev/null; then
        pass "Secret ${secret}: valid until ${EXPIRY}"
      else
        fail "Secret ${secret}: EXPIRED (was ${EXPIRY})"
      fi
    fi
  done <<< "${CERT_SECRETS}"
fi

# ── 12. Recent warning events ─────────────────────────────────────────────────
header "12. Recent Warning Events (namespace: ${NAMESPACE})"

EVENTS=$(kubectl get events -n "${NAMESPACE}" \
  --field-selector type=Warning \
  --sort-by='.lastTimestamp' \
  -o jsonpath='{range .items[-10:]}{.lastTimestamp}{"\t"}{.involvedObject.name}{"\t"}{.reason}{"\t"}{.message}{"\n"}{end}' \
  2>/dev/null || true)

if [[ -z "${EVENTS}" ]]; then
  pass "No Warning events in namespace ${NAMESPACE}"
else
  echo ""
  printf "  %-25s %-30s %-20s %s\n" "TIME" "OBJECT" "REASON" "MESSAGE"
  printf "  %-25s %-30s %-20s %s\n" "----" "------" "------" "-------"
  while IFS=$'\t' read -r ts obj reason msg; do
    printf "  %-25s %-30s %-20s %s\n" \
      "${ts:0:19}" "${obj:0:29}" "${reason:0:19}" "${msg:0:60}"
  done <<< "${EVENTS}"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
header "Summary"
echo ""
echo -e "  Namespace checked : ${BOLD}${NAMESPACE}${RESET}"
[[ -n "${NODE_NAME}" ]]     && echo -e "  Node targeted     : ${BOLD}${NODE_NAME}${RESET}"
[[ -n "${EXTERNAL_PEER}" ]] && echo -e "  External peer     : ${BOLD}${EXTERNAL_PEER}${RESET}"
echo ""
echo -e "  For detailed troubleshooting see:"
echo -e "  ${CYAN}docs/troubleshooting/networking.md${RESET}"
echo ""
