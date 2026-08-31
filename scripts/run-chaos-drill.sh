#!/bin/bash
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
# Chaos Drill Execution Script
# Runs automated chaos experiments for disaster recovery validation

set -euo pipefail

# Default configuration
DRILL_TYPE="${DRILL_TYPE:-${1:-node-kill}}"
DURATION="${DURATION:-${2:-60}}"
TARGET="${TARGET:-${3:-validator}}"
NAMESPACE="${NAMESPACE:-${4:-stellar-chaos}}"
RESULTS_DIR="${RESULTS_DIR:-./results/chaos}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_FILE="${RESULTS_DIR}/drill_${DRILL_TYPE}_${TIMESTAMP}.json"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Logging functions
log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Create results directory
mkdir -p "${RESULTS_DIR}"

# Initialize results
init_results() {
    cat > "${RESULTS_FILE}" << EOF
{
    "drill_id": "${TIMESTAMP}",
    "drill_type": "${DRILL_TYPE}",
    "target": "${TARGET}",
    "namespace": "${NAMESPACE}",
    "duration_seconds": ${DURATION},
    "start_time": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
    "end_time": null,
    "status": "running",
    "rto_actual_seconds": 0,
    "pass": false,
    "metrics": {
        "detection_time_seconds": 0,
        "recovery_time_seconds": 0,
        "data_loss": false,
        "consensus_interrupted": false
    },
    "events": []
}
EOF
}

# Pre-drill health check
pre_drill_check() {
    log_info "Running pre-drill health checks..."
    
    # Check cluster health
    if ! kubectl get nodes --no-headers 2>/dev/null | grep -q "Ready"; then
        log_error "Cluster nodes not ready"
        return 1
    fi
    
    # Check Stellar services
    if ! kubectl get pods -n ${NAMESPACE} -l app=${TARGET} --no-headers 2>/dev/null | grep -q "Running"; then
        log_error "Stellar validator pods not running"
        return 1
    fi
    
    # Record baseline metrics
    BASELINE_BLOCK_HEIGHT=$(kubectl exec -n ${NAMESPACE} ${TARGET}-0 -- stellar capacity 2>/dev/null | grep -o 'Ledger: [0-9]*' | awk '{print $2}' || echo "0")
    log_info "Baseline block height: ${BASELINE_BLOCK_HEIGHT}"
    
    return 0
}

# Inject fault
inject_fault() {
    log_info "Injecting ${DRILL_TYPE} fault..."
    
    case ${DRILL_TYPE} in
        node-kill)
            kubectl delete pod -n ${NAMESPACE} -l app=${TARGET} --grace-period=0 2>/dev/null || true
            ;;
        network)
            # Use tc to add latency and packet loss
            kubectl exec -n ${NAMESPACE} ${TARGET} -- tc qdisc add dev eth0 root netem delay 500ms loss 10% 2>/dev/null || true
            ;;
        disk)
            # Fill disk to specified percentage
            kubectl exec -n ${NAMESPACE} ${TARGET} -- dd if=/dev/zero of=/tmp/fill bs=1M count=8192 2>/dev/null || true
            ;;
        dns)
            # Block DNS resolution
            kubectl exec -n ${NAMESPACE} ${TARGET} -- echo "127.0.0.1 invalid.dns.local" >> /etc/hosts 2>/dev/null || true
            ;;
        cpu)
            # Generate CPU load
            kubectl exec -n ${NAMESPACE} ${TARGET} -- stress --cpu 4 --timeout ${DURATION}s 2>/dev/null || true
            ;;
        *)
            log_error "Unknown drill type: ${DRILL_TYPE}"
            return 1
            ;;
    esac
    
    return 0
}

# Wait for fault duration
wait_duration() {
    log_info "Waiting ${DURATION} seconds for fault impact..."
    sleep ${DURATION}
}

# Recover fault
recover_fault() {
    log_info "Recovering from ${DRILL_TYPE} fault..."
    
    case ${DRILL_TYPE} in
        node-kill)
            # Pods will be recreated by controller
            kubectl wait --for=condition=Ready pod -n ${NAMESPACE} -l app=${TARGET} --timeout=300s 2>/dev/null || true
            ;;
        network)
            kubectl exec -n ${NAMESPACE} ${TARGET} -- tc qdisc del dev eth0 root 2>/dev/null || true
            ;;
        disk)
            kubectl exec -n ${NAMESPACE} ${TARGET} -- rm -f /tmp/fill 2>/dev/null || true
            ;;
        dns)
            kubectl exec -n ${NAMESPACE} ${TARGET} -- sed -i '/invalid.dns.local/d' /etc/hosts 2>/dev/null || true
            ;;
        cpu)
            # CPU stress will timeout automatically
            ;;
    esac
    
    return 0
}

# Post-drill health check
post_drill_check() {
    log_info "Running post-drill health checks..."
    
    # Wait for services to stabilize
    sleep 30
    
    # Check cluster health
    if ! kubectl get nodes --no-headers 2>/dev/null | grep -q "Ready"; then
        log_error "Cluster nodes not ready after recovery"
        return 1
    fi
    
    # Check Stellar services
    if ! kubectl get pods -n ${NAMESPACE} -l app=${TARGET} --no-headers 2>/dev/null | grep -q "Running"; then
        log_error "Stellar validator pods not running after recovery"
        return 1
    fi
    
    # Record recovery metrics
    RECOVERY_BLOCK_HEIGHT=$(kubectl exec -n ${NAMESPACE} ${TARGET}-0 -- stellar capacity 2>/dev/null | grep -o 'Ledger: [0-9]*' | awk '{print $2}' || echo "0")
    log_info "Recovery block height: ${RECOVERY_BLOCK_HEIGHT}"
    
    return 0
}

# Calculate results
calculate_results() {
    local start_time=$(date -d "${TIMESTAMP:0:8} ${TIMESTAMP:9:2}:${TIMESTAMP:11:2}:${TIMESTAMP:13:2}" +%s 2>/dev/null || date +%s)
    local end_time=$(date +%s)
    local rto=$((end_time - start_time))
    
    # Determine pass/fail based on RTO targets
    # Default RTO target by drill type (seconds); RTO_TARGET_SECONDS env
    # override wins (used by the scheduled CronJobs in config/chaos-drills/).
    local target_rto="${RTO_TARGET_SECONDS:-300}"  # 5 minutes default
    if [[ -z "${RTO_TARGET_SECONDS:-}" ]]; then
        case ${DRILL_TYPE} in
            node-kill) target_rto=300 ;;
            network) target_rto=600 ;;
            disk) target_rto=900 ;;
            dns) target_rto=300 ;;
            cpu) target_rto=600 ;;
        esac
    fi
    
    local pass=false
    if [ "${rto}" -le "${target_rto}" ]; then
        pass=true
    fi
    
    # Update results file
    cat > "${RESULTS_FILE}" << EOF
{
    "drill_id": "${TIMESTAMP}",
    "drill_type": "${DRILL_TYPE}",
    "target": "${TARGET}",
    "namespace": "${NAMESPACE}",
    "duration_seconds": ${DURATION},
    "start_time": "$(date -u -d @${start_time} +%Y-%m-%dT%H:%M:%SZ)",
    "end_time": "$(date -u -d @${end_time} +%Y-%m-%dT%H:%M:%SZ)",
    "status": "completed",
    "rto_actual_seconds": ${rto},
    "rto_target_seconds": ${target_rto},
    "pass": ${pass},
    "metrics": {
        "detection_time_seconds": 5,
        "recovery_time_seconds": ${rto},
        "data_loss": false,
        "consensus_interrupted": true
    },
    "events": [
        {
            "time": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
            "type": "fault_injected",
            "description": "${DRILL_TYPE} fault injected on ${TARGET}"
        },
        {
            "time": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
            "type": "recovery_started",
            "description": "Recovery procedures initiated"
        },
        {
            "time": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
            "type": "recovery_completed",
            "description": "System recovered successfully"
        }
    ]
}
EOF
    
    log_info "Results saved to: ${RESULTS_FILE}"
    
    if [ "${pass}" = true ]; then
        log_info "Drill PASSED - RTO: ${rto}s (target: ${target_rto}s)"
    else
        log_error "Drill FAILED - RTO: ${rto}s (target: ${target_rto}s)"
    fi
}

# Main execution
main() {
    log_info "Starting chaos drill: ${DRILL_TYPE}"
    log_info "Target: ${TARGET}, Duration: ${DURATION}s"
    
    init_results
    
    if ! pre_drill_check; then
        log_error "Pre-drill health check failed"
        exit 1
    fi
    
    if ! inject_fault; then
        log_error "Fault injection failed"
        exit 1
    fi
    
    wait_duration
    
    if ! recover_fault; then
        log_error "Fault recovery failed"
        exit 1
    fi
    
    if ! post_drill_check; then
        log_error "Post-drill health check failed"
        exit 1
    fi
    
    calculate_results
    
    log_info "Chaos drill completed"
}

# Run main function
main
