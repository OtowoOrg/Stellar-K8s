#!/usr/bin/env bash
# reset-captive-core.sh
#
# Safely wipes the Captive Core ephemeral state directory from a Horizon pod
# and restarts the deployment so Horizon can rebuild ledger state from network
# history archives.
#
# This script targets ONLY the Captive Core state directory. It will NOT:
#   - Delete the Horizon PostgreSQL database
#   - Modify any PersistentVolumeClaims
#   - Touch validator seed secrets or signing keys
#
# Usage:
#   NS=stellar HORIZON_DEPLOY=horizon bash examples/troubleshooting/reset-captive-core.sh
#
# Required environment variables (or flags):
#   NS                  Kubernetes namespace (default: stellar)
#   HORIZON_DEPLOY      Horizon Deployment name (default: horizon)
#   CAPTIVE_CORE_DIR    Captive Core state directory path (default: /var/lib/stellar)
#
# Optional environment variables:
#   DATABASE_URL        PostgreSQL DSN used to verify DB health pre-wipe
#   DRY_RUN             Set to "true" to preview actions without executing (default: false)
#   SKIP_DB_CHECK       Set to "true" to skip the PostgreSQL pre-flight check
#   STELLAR_NODE_NAME   If set, places the StellarNode CRD in maintenance mode
#
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

set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# Configuration
# ─────────────────────────────────────────────────────────────────────────────
readonly SCRIPT_NAME="$(basename "$0")"
readonly SCRIPT_VERSION="1.0.0"

NS="${NS:-stellar}"
HORIZON_DEPLOY="${HORIZON_DEPLOY:-horizon}"
CAPTIVE_CORE_DIR="${CAPTIVE_CORE_DIR:-/var/lib/stellar}"
DATABASE_URL="${DATABASE_URL:-}"
DRY_RUN="${DRY_RUN:-false}"
SKIP_DB_CHECK="${SKIP_DB_CHECK:-false}"
STELLAR_NODE_NAME="${STELLAR_NODE_NAME:-}"
TIMEOUT_SCALE_DOWN="${TIMEOUT_SCALE_DOWN:-120}"
TIMEOUT_SCALE_UP="${TIMEOUT_SCALE_UP:-300}"

# ─────────────────────────────────────────────────────────────────────────────
# Colors and logging
# ─────────────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

log_info()    { echo -e "${GREEN}[INFO]${NC}  $*"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
log_error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; }
log_step()    { echo -e "\n${BLUE}${BOLD}==> $*${NC}"; }
log_dry_run() { echo -e "${YELLOW}[DRY-RUN]${NC} Would execute: $*"; }

# ─────────────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────────────

# run_cmd: execute or dry-run a command
run_cmd() {
    if [[ "$DRY_RUN" == "true" ]]; then
        log_dry_run "$*"
        return 0
    fi
    "$@"
}

# require_cmd: abort if a required binary is missing
require_cmd() {
    local cmd="$1"
    if ! command -v "$cmd" &>/dev/null; then
        log_error "Required command not found: ${cmd}"
        exit 1
    fi
}

# get_horizon_pod: retrieve the first running Horizon pod name
get_horizon_pod() {
    kubectl get pod -n "$NS" \
      -l "app.kubernetes.io/name=horizon" \
      --field-selector=status.phase=Running \
      -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true
}

# confirm: ask the user to confirm a destructive action (skipped in dry-run)
confirm() {
    local prompt="$1"
    if [[ "$DRY_RUN" == "true" ]]; then
        log_dry_run "Skipping confirmation: ${prompt}"
        return 0
    fi
    echo -e "${YELLOW}${BOLD}[CONFIRM]${NC} ${prompt}"
    read -r -p "  Type 'yes' to proceed, anything else to abort: " answer
    if [[ "$answer" != "yes" ]]; then
        log_warn "Aborted by user."
        exit 0
    fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Argument / flag parsing
# ─────────────────────────────────────────────────────────────────────────────
usage() {
    cat <<EOF
${SCRIPT_NAME} v${SCRIPT_VERSION} — Captive Core state reset tool

USAGE:
  NS=<ns> HORIZON_DEPLOY=<name> bash ${SCRIPT_NAME} [--dry-run] [--skip-db-check] [--help]

FLAGS:
  --dry-run         Preview actions without executing any kubectl commands
  --skip-db-check   Skip the PostgreSQL connectivity pre-flight
  --help            Show this help message

ENVIRONMENT VARIABLES:
  NS                     Kubernetes namespace (default: stellar)
  HORIZON_DEPLOY         Horizon Deployment name (default: horizon)
  CAPTIVE_CORE_DIR       Captive Core state path (default: /var/lib/stellar)
  DATABASE_URL           Horizon PostgreSQL DSN (optional, enables DB check)
  STELLAR_NODE_NAME      StellarNode CRD name (optional, enables maintenance mode)
  TIMEOUT_SCALE_DOWN     Seconds to wait for scale-down (default: 120)
  TIMEOUT_SCALE_UP       Seconds to wait for scale-up (default: 300)

EXAMPLES:
  # Dry-run to preview what would happen
  NS=stellar DRY_RUN=true bash examples/troubleshooting/reset-captive-core.sh

  # Full reset with CRD maintenance mode
  NS=stellar HORIZON_DEPLOY=horizon STELLAR_NODE_NAME=horizon \\
    bash examples/troubleshooting/reset-captive-core.sh

  # Custom captive core path
  NS=stellar CAPTIVE_CORE_DIR=/mnt/stellar-data/captive-core \\
    bash examples/troubleshooting/reset-captive-core.sh

SEE ALSO:
  docs/operations/captive-core-rebuild.md
EOF
}

for arg in "$@"; do
    case "$arg" in
        --dry-run)        DRY_RUN=true ;;
        --skip-db-check)  SKIP_DB_CHECK=true ;;
        --help|-h)        usage; exit 0 ;;
        *)
            log_error "Unknown argument: ${arg}"
            usage
            exit 1
            ;;
    esac
done

# ─────────────────────────────────────────────────────────────────────────────
# Pre-flight checks
# ─────────────────────────────────────────────────────────────────────────────
preflight_checks() {
    log_step "Pre-flight checks"

    require_cmd kubectl

    # Confirm namespace exists
    if ! kubectl get namespace "$NS" &>/dev/null; then
        log_error "Namespace '${NS}' not found."
        exit 1
    fi
    log_info "Namespace: ${NS}"

    # Confirm Horizon deployment exists
    if ! kubectl get deployment "$HORIZON_DEPLOY" -n "$NS" &>/dev/null; then
        log_error "Deployment '${HORIZON_DEPLOY}' not found in namespace '${NS}'."
        exit 1
    fi
    log_info "Horizon deployment: ${HORIZON_DEPLOY}"

    # Safety: validate CAPTIVE_CORE_DIR is never empty or root
    if [[ -z "$CAPTIVE_CORE_DIR" || "$CAPTIVE_CORE_DIR" == "/" ]]; then
        log_error "CAPTIVE_CORE_DIR is empty or '/' — refusing to proceed."
        exit 1
    fi
    log_info "Captive Core state directory: ${CAPTIVE_CORE_DIR}"

    # PostgreSQL pre-flight
    if [[ "$SKIP_DB_CHECK" == "false" && -n "$DATABASE_URL" ]]; then
        log_info "Checking Horizon PostgreSQL database connectivity..."
        local pod
        pod=$(get_horizon_pod)
        if [[ -z "$pod" ]]; then
            log_warn "No running Horizon pod found; skipping PostgreSQL check."
        else
            if kubectl exec "$pod" -n "$NS" -c horizon -- \
                psql "${DATABASE_URL}" -c "SELECT COUNT(*) FROM history_ledgers;" \
                &>/dev/null; then
                log_info "PostgreSQL check passed — Horizon database is healthy."
            else
                log_error "PostgreSQL check FAILED. This may not be a Captive Core issue."
                log_error "Investigate the Horizon database before proceeding."
                log_error "Override with SKIP_DB_CHECK=true if you are certain the DB is healthy."
                exit 1
            fi
        fi
    elif [[ "$SKIP_DB_CHECK" == "true" ]]; then
        log_warn "Skipping PostgreSQL pre-flight check (SKIP_DB_CHECK=true)."
    else
        log_warn "DATABASE_URL not set; skipping PostgreSQL pre-flight check."
    fi

    log_info "Pre-flight checks passed."
}

# ─────────────────────────────────────────────────────────────────────────────
# Phase 1: Enable maintenance mode (optional, CRD-managed only)
# ─────────────────────────────────────────────────────────────────────────────
enable_maintenance_mode() {
    if [[ -z "$STELLAR_NODE_NAME" ]]; then
        return 0
    fi
    log_step "Enabling StellarNode maintenance mode: ${STELLAR_NODE_NAME}"

    if ! kubectl get stellarnode "$STELLAR_NODE_NAME" -n "$NS" &>/dev/null; then
        log_warn "StellarNode '${STELLAR_NODE_NAME}' not found — skipping maintenance mode."
        return 0
    fi

    run_cmd kubectl patch stellarnode "$STELLAR_NODE_NAME" -n "$NS" \
        --type=merge -p '{"spec":{"maintenanceMode":true}}'

    if [[ "$DRY_RUN" != "true" ]]; then
        log_info "Waiting for Maintenance phase..."
        local retries=0
        while [[ $retries -lt 15 ]]; do
            local phase
            phase=$(kubectl get stellarnode "$STELLAR_NODE_NAME" -n "$NS" \
                -o jsonpath='{.status.phase}' 2>/dev/null || echo "Unknown")
            if [[ "$phase" == "Maintenance" ]]; then
                log_info "StellarNode is in Maintenance phase."
                return 0
            fi
            retries=$((retries + 1))
            sleep 2
        done
        log_warn "StellarNode did not reach Maintenance phase within 30s; continuing anyway."
    fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Phase 2: Scale down Horizon
# ─────────────────────────────────────────────────────────────────────────────
scale_down_horizon() {
    log_step "Scaling down Horizon deployment to 0 replicas"

    local current_replicas
    current_replicas=$(kubectl get deployment "$HORIZON_DEPLOY" -n "$NS" \
        -o jsonpath='{.spec.replicas}' 2>/dev/null || echo "0")
    log_info "Current replicas: ${current_replicas}"

    # Persist original replica count for restoration
    echo "$current_replicas" > /tmp/horizon-original-replicas.txt

    run_cmd kubectl scale deployment "$HORIZON_DEPLOY" -n "$NS" --replicas=0

    if [[ "$DRY_RUN" != "true" ]]; then
        log_info "Waiting for all pods to terminate (timeout: ${TIMEOUT_SCALE_DOWN}s)..."
        kubectl wait deployment "$HORIZON_DEPLOY" -n "$NS" \
            --for=jsonpath='{.status.availableReplicas}'=0 \
            --timeout="${TIMEOUT_SCALE_DOWN}s" 2>/dev/null || {
            # The field disappears entirely when there are 0 replicas; that's also success
            local remaining
            remaining=$(kubectl get pod -n "$NS" \
                -l "app.kubernetes.io/name=horizon" \
                --no-headers 2>/dev/null | wc -l)
            if [[ "$remaining" -gt 0 ]]; then
                log_error "Horizon pods did not terminate within ${TIMEOUT_SCALE_DOWN}s."
                log_error "Check for pod disruption budgets or stuck finalizers."
                exit 1
            fi
        }
        log_info "All Horizon pods terminated."
    fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Phase 3: Wipe Captive Core state directory
# ─────────────────────────────────────────────────────────────────────────────
wipe_captive_core_state() {
    log_step "Wiping Captive Core state directory: ${CAPTIVE_CORE_DIR}"

    # Construct the wipe command carefully to avoid path issues
    # Double-checking CAPTIVE_CORE_DIR is not empty or root (belt-and-suspenders)
    local safe_dir="${CAPTIVE_CORE_DIR:?CAPTIVE_CORE_DIR must not be empty}"
    if [[ "$safe_dir" == "/" || "$safe_dir" == "/var" || "$safe_dir" == "/mnt" ]]; then
        log_error "CAPTIVE_CORE_DIR '${safe_dir}' is a dangerous path — refusing to delete."
        exit 1
    fi

    # The wipe runs as a one-shot pod that mounts the Horizon PVC
    # Try to find the Horizon PVC name
    local pvc_name
    pvc_name=$(kubectl get pvc -n "$NS" \
        -l "app.kubernetes.io/name=horizon" \
        -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)

    if [[ -n "$pvc_name" ]]; then
        log_info "Wiping via maintenance pod (PVC: ${pvc_name})"
        wipe_via_maintenance_pod "$pvc_name" "$safe_dir"
    else
        log_warn "No labeled Horizon PVC found. Attempting wipe via a restarted pod exec."
        log_warn "If the pod fails to start, set the PVC name manually and retry."
        wipe_via_pod_exec "$safe_dir"
    fi
}

# Wipe via a one-shot Pod that mounts the Horizon PVC
wipe_via_maintenance_pod() {
    local pvc="$1"
    local dir="$2"
    local pod_name="captive-core-wipe-$(date +%s)"

    # Determine mount path: if captive core dir is under /mnt, mount there
    local mount_path
    if [[ "$dir" == /mnt/* ]]; then
        mount_path="/mnt"
    else
        mount_path="/data"
    fi
    local wipe_path="${mount_path}${dir#/var}"
    # If captive core dir is /var/lib/stellar, we need to mount / adjust path
    # Simplest: mount PVC at same prefix
    local adjusted_dir="${dir}"

    log_info "Launching one-shot maintenance pod: ${pod_name}"

    local wipe_manifest
    wipe_manifest=$(cat <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: ${pod_name}
  namespace: ${NS}
  labels:
    app.kubernetes.io/name: captive-core-wipe
    app.kubernetes.io/component: recovery
spec:
  restartPolicy: Never
  terminationGracePeriodSeconds: 5
  volumes:
    - name: horizon-data
      persistentVolumeClaim:
        claimName: ${pvc}
  containers:
    - name: wipe
      image: busybox:1.36
      imagePullPolicy: IfNotPresent
      command:
        - sh
        - -c
        - |
          set -e
          TARGET="\${CAPTIVE_CORE_DIR}"
          echo "Removing Captive Core state: \${TARGET}"
          if [ -d "\${TARGET}" ]; then
            rm -rf "\${TARGET}"
            echo "Directory removed: \${TARGET}"
          else
            echo "Directory does not exist (already clean): \${TARGET}"
          fi
          mkdir -p "\${TARGET}"
          echo "CAPTIVE_CORE_WIPE_COMPLETE"
      env:
        - name: CAPTIVE_CORE_DIR
          value: "${adjusted_dir}"
      volumeMounts:
        - name: horizon-data
          mountPath: /mnt/horizon-data
      resources:
        requests:
          cpu: 50m
          memory: 32Mi
EOF
)

    if [[ "$DRY_RUN" == "true" ]]; then
        log_dry_run "Would apply maintenance pod manifest (pod: ${pod_name})"
        log_dry_run "Would exec: rm -rf ${adjusted_dir}/ && mkdir -p ${adjusted_dir}"
        return 0
    fi

    echo "$wipe_manifest" | kubectl apply -f -

    log_info "Waiting for wipe pod to complete..."
    local retries=0
    while [[ $retries -lt 60 ]]; do
        local phase
        phase=$(kubectl get pod "$pod_name" -n "$NS" \
            -o jsonpath='{.status.phase}' 2>/dev/null || echo "Pending")
        if [[ "$phase" == "Succeeded" ]]; then
            log_info "Wipe pod completed successfully."
            break
        elif [[ "$phase" == "Failed" ]]; then
            log_error "Wipe pod failed. Logs:"
            kubectl logs "$pod_name" -n "$NS" >&2
            kubectl delete pod "$pod_name" -n "$NS" --ignore-not-found=true
            exit 1
        fi
        retries=$((retries + 1))
        sleep 3
    done

    if [[ $retries -ge 60 ]]; then
        log_error "Wipe pod did not complete within 180s."
        kubectl delete pod "$pod_name" -n "$NS" --ignore-not-found=true
        exit 1
    fi

    # Print wipe pod logs as confirmation
    kubectl logs "$pod_name" -n "$NS"
    # Clean up
    kubectl delete pod "$pod_name" -n "$NS" --ignore-not-found=true
    log_info "Maintenance pod deleted."
}

# Fallback: wipe by executing into the Horizon pod during a brief restart window
wipe_via_pod_exec() {
    local dir="$1"
    log_warn "Attempting to exec into a short-lived Horizon pod for wipe."
    log_warn "This may fail if no pod is currently running."

    local pod
    pod=$(get_horizon_pod)
    if [[ -z "$pod" ]]; then
        log_error "No running Horizon pod found."
        log_error "Set STELLAR_NODE_NAME to have the script manage the CRD, or scale"
        log_error "the deployment manually, exec in, and delete: rm -rf ${dir}"
        exit 1
    fi

    run_cmd kubectl exec "$pod" -n "$NS" -c horizon -- \
        sh -c "rm -rf \"${dir:?}\" && mkdir -p \"${dir}\" && echo 'CAPTIVE_CORE_WIPE_COMPLETE'"
}

# ─────────────────────────────────────────────────────────────────────────────
# Phase 4: Scale Horizon back up
# ─────────────────────────────────────────────────────────────────────────────
scale_up_horizon() {
    log_step "Scaling Horizon deployment back up"

    local target_replicas=1
    if [[ -f /tmp/horizon-original-replicas.txt ]]; then
        target_replicas=$(cat /tmp/horizon-original-replicas.txt)
        rm -f /tmp/horizon-original-replicas.txt
        # Default to at least 1
        if [[ "$target_replicas" -lt 1 ]]; then
            target_replicas=1
        fi
    fi
    log_info "Scaling to ${target_replicas} replica(s)"

    # If StellarNode-managed, disable maintenance mode before scaling
    if [[ -n "$STELLAR_NODE_NAME" ]]; then
        log_info "Disabling StellarNode maintenance mode..."
        run_cmd kubectl patch stellarnode "$STELLAR_NODE_NAME" -n "$NS" \
            --type=merge -p '{"spec":{"maintenanceMode":false}}'
    fi

    run_cmd kubectl scale deployment "$HORIZON_DEPLOY" -n "$NS" \
        --replicas="$target_replicas"

    if [[ "$DRY_RUN" != "true" ]]; then
        log_info "Waiting for rollout (timeout: ${TIMEOUT_SCALE_UP}s)..."
        kubectl rollout status deployment/"$HORIZON_DEPLOY" -n "$NS" \
            --timeout="${TIMEOUT_SCALE_UP}s"
        log_info "Horizon is running."
    fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Phase 5: Post-restart verification
# ─────────────────────────────────────────────────────────────────────────────
verify_recovery() {
    log_step "Verifying Captive Core recovery"

    if [[ "$DRY_RUN" == "true" ]]; then
        log_dry_run "Would check Horizon pod status and catchup logs."
        return 0
    fi

    # Wait a moment for the process to start
    sleep 10

    local pod
    pod=$(get_horizon_pod)
    if [[ -z "$pod" ]]; then
        log_warn "No running Horizon pod found yet. It may still be starting."
        log_warn "Check: kubectl logs deployment/${HORIZON_DEPLOY} -n ${NS} -c horizon"
        return 0
    fi

    log_info "Checking Horizon logs for Captive Core startup..."
    kubectl logs "$pod" -n "$NS" -c horizon --tail=30 | \
        grep -E "(captive|catchup|ingest|state)" || true

    log_info "Checking Horizon root endpoint..."
    local state
    state=$(kubectl exec "$pod" -n "$NS" -c horizon -- \
        wget -qO- http://localhost:8000/ 2>/dev/null | \
        python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('state','unknown'))" \
        2>/dev/null || echo "unavailable")
    log_info "Horizon state: ${state}"

    if [[ "$state" == "synced" ]]; then
        log_info "${GREEN}✓ Horizon is synced. Captive Core rebuild complete.${NC}"
    elif [[ "$state" == "syncing" ]]; then
        log_info "${YELLOW}⏳ Horizon is catching up. Monitor with:${NC}"
        log_info "  kubectl logs -f deployment/${HORIZON_DEPLOY} -n ${NS} -c horizon"
    else
        log_warn "Horizon state is '${state}'. Monitor manually:"
        log_warn "  kubectl logs -f deployment/${HORIZON_DEPLOY} -n ${NS} -c horizon"
    fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Summary banner
# ─────────────────────────────────────────────────────────────────────────────
print_summary() {
    echo ""
    echo -e "${BOLD}────────────────────────────────────────────────────────────${NC}"
    echo -e "${BOLD}  Captive Core Reset Summary${NC}"
    echo -e "${BOLD}────────────────────────────────────────────────────────────${NC}"
    echo -e "  Namespace:        ${NS}"
    echo -e "  Horizon Deploy:   ${HORIZON_DEPLOY}"
    echo -e "  Captive Core Dir: ${CAPTIVE_CORE_DIR}"
    if [[ "$DRY_RUN" == "true" ]]; then
        echo -e "  Mode:             ${YELLOW}DRY RUN — no changes were made${NC}"
    else
        echo -e "  Mode:             ${GREEN}EXECUTED${NC}"
    fi
    echo -e "${BOLD}────────────────────────────────────────────────────────────${NC}"
    echo ""
    echo -e "  Next steps:"
    echo -e "  1. Monitor catchup: kubectl logs -f deployment/${HORIZON_DEPLOY} -n ${NS} -c horizon"
    echo -e "  2. Check state:     kubectl exec -n ${NS} deploy/${HORIZON_DEPLOY} -- wget -qO- http://localhost:8000/ | python3 -m json.tool | grep state"
    echo -e "  3. Read the guide:  docs/operations/captive-core-rebuild.md"
    echo ""
    echo -e "  ${YELLOW}⚠️  The Horizon PostgreSQL database was NOT touched by this script.${NC}"
    echo ""
}

# ─────────────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────────────
main() {
    echo -e "${BOLD}${BLUE}reset-captive-core.sh v${SCRIPT_VERSION}${NC}"
    echo -e "Captive Core state reset for Horizon (Issue #252)"
    echo ""

    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "DRY-RUN MODE: no destructive actions will be taken."
    fi

    preflight_checks

    # Safety confirmation before any destructive action
    confirm "This will scale down Horizon and wipe ${CAPTIVE_CORE_DIR} on namespace '${NS}'."

    enable_maintenance_mode
    scale_down_horizon
    wipe_captive_core_state
    scale_up_horizon
    verify_recovery
    print_summary
}

main "$@"
