#!/usr/bin/env bash
#
# Webhook Performance Benchmark Runner
#
# This script runs the webhook performance benchmarks and generates reports.
# It can be run locally or in CI/CD pipelines.

set -euo pipefail

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RESULTS_DIR="${PROJECT_ROOT}/results"
BASELINE_FILE="${SCRIPT_DIR}/baselines/webhook-v0.1.0.json"
K6_SCRIPT="${SCRIPT_DIR}/k6/webhook-load-test.js"

# Environment variables
WEBHOOK_URL="${WEBHOOK_URL:-http://localhost:8443}"
VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null || echo 'unknown')}"
GIT_SHA="${GIT_SHA:-$(git rev-parse HEAD 2>/dev/null || echo 'unknown')}"
RUN_ID="${RUN_ID:-local-$(date +%s)}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

check_dependencies() {
    log_info "Checking dependencies..."
    
    if ! command -v k6 &> /dev/null; then
        log_error "k6 is not installed. Please install it from https://k6.io/docs/getting-started/installation/"
        exit 1
    fi
    
    log_success "All dependencies are installed"
}

check_webhook_health() {
    log_info "Checking webhook health at ${WEBHOOK_URL}..."
    
    if curl -sf "${WEBHOOK_URL}/health" > /dev/null 2>&1; then
        log_success "Webhook is healthy"
        return 0
    else
        log_warning "Webhook health check failed. Make sure the webhook server is running."
        log_info "You can start it with: cargo run --bin stellar-operator webhook"
        return 1
    fi
}

create_results_dir() {
    mkdir -p "${RESULTS_DIR}"
    log_info "Results will be saved to: ${RESULTS_DIR}"
}

run_benchmark() {
    local scenario="${1:-both}"
    
    log_info "Running webhook benchmark (scenario: ${scenario})..."
    log_info "Webhook URL: ${WEBHOOK_URL}"
    log_info "Version: ${VERSION}"
    log_info "Git SHA: ${GIT_SHA}"
    log_info "Run ID: ${RUN_ID}"
    
    # Run k6 with environment variables
    k6 run \
        --out "json=${RESULTS_DIR}/webhook-benchmark-raw.json" \
        --env "WEBHOOK_URL=${WEBHOOK_URL}" \
        --env "VERSION=${VERSION}" \
        --env "GIT_SHA=${GIT_SHA}" \
        --env "RUN_ID=${RUN_ID}" \
        --env "BASELINE_FILE=${BASELINE_FILE}" \
        --env "SCENARIO=${scenario}" \
        "${K6_SCRIPT}"
    
    local exit_code=$?
    
    if [ $exit_code -eq 0 ]; then
        log_success "Benchmark completed successfully"
    else
        log_error "Benchmark failed with exit code ${exit_code}"
        return $exit_code
    fi
}

generate_comparison_report() {
    log_info "Generating comparison report..."
    
    local summary_file="${RESULTS_DIR}/webhook-benchmark.json"
    
    if [ ! -f "${summary_file}" ]; then
        log_warning "Summary file not found: ${summary_file}"
        return 1
    fi
    
    # Use jq to compare with baseline if available
    if command -v jq &> /dev/null && [ -f "${BASELINE_FILE}" ]; then
        log_info "Comparing results with baseline..."
        
        local val_p99=$(jq -r '.webhook_metrics.validation_p99' "${summary_file}")
        local mut_p99=$(jq -r '.webhook_metrics.mutation_p99' "${summary_file}")
        local throughput=$(jq -r '.webhook_metrics.throughput' "${summary_file}")
        
        local baseline_val_p99=$(jq -r '.webhook_metrics.validation_p99' "${BASELINE_FILE}")
        local baseline_mut_p99=$(jq -r '.webhook_metrics.mutation_p99' "${BASELINE_FILE}")
        local baseline_throughput=$(jq -r '.webhook_metrics.throughput' "${BASELINE_FILE}")
        
        echo ""
        echo "==================================================================="
        echo "  PERFORMANCE COMPARISON"
        echo "==================================================================="
        echo ""
        printf "%-30s %10s %10s %10s\n" "Metric" "Current" "Baseline" "Change"
        echo "-------------------------------------------------------------------"
        printf "%-30s %9.2fms %9.2fms %+9.1f%%\n" "Validation p99" "$val_p99" "$baseline_val_p99" \
            "$(echo "scale=1; ($val_p99 - $baseline_val_p99) / $baseline_val_p99 * 100" | bc)"
        printf "%-30s %9.2fms %9.2fms %+9.1f%%\n" "Mutation p99" "$mut_p99" "$baseline_mut_p99" \
            "$(echo "scale=1; ($mut_p99 - $baseline_mut_p99) / $baseline_mut_p99 * 100" | bc)"
        printf "%-30s %8.2f/s %8.2f/s %+9.1f%%\n" "Throughput" "$throughput" "$baseline_throughput" \
            "$(echo "scale=1; ($throughput - $baseline_throughput) / $baseline_throughput * 100" | bc)"
        echo "==================================================================="
        echo ""
        
        # Check for regressions
        local regression_detected=$(jq -r '.regression.detected' "${summary_file}")
        if [ "${regression_detected}" = "true" ]; then
            log_error "REGRESSION DETECTED!"
            return 1
        else
            log_success "No regressions detected"
        fi
    else
        log_warning "jq not installed or baseline not found, skipping comparison"
    fi
}

display_results() {
    log_info "Benchmark results:"
    echo ""
    
    if [ -f "${RESULTS_DIR}/webhook-benchmark-report.md" ]; then
        cat "${RESULTS_DIR}/webhook-benchmark-report.md"
    else
        log_warning "Markdown report not found"
    fi
}

save_as_baseline() {
    local summary_file="${RESULTS_DIR}/webhook-benchmark.json"
    
    if [ ! -f "${summary_file}" ]; then
        log_error "Cannot save baseline: summary file not found"
        return 1
    fi
    
    local new_baseline="${SCRIPT_DIR}/baselines/webhook-${VERSION}.json"
    
    log_info "Saving current results as new baseline: ${new_baseline}"
    cp "${summary_file}" "${new_baseline}"
    
    log_success "Baseline saved"
}

# Main execution
main() {
    local command="${1:-run}"
    shift || true
    
    case "${command}" in
        run)
            check_dependencies
            create_results_dir
            
            if ! check_webhook_health; then
                log_error "Webhook is not accessible. Exiting."
                exit 1
            fi
            
            run_benchmark "$@"
            generate_comparison_report
            ;;
        
        compare)
            generate_comparison_report
            ;;
        
        display)
            display_results
            ;;
        
        save-baseline)
            save_as_baseline
            ;;
        
        health)
            check_webhook_health
            ;;
        
        *)
            echo "Usage: $0 {run|compare|display|save-baseline|health} [scenario]"
            echo ""
            echo "Commands:"
            echo "  run [scenario]    Run the benchmark (scenarios: both, validate, mutate)"
            echo "  compare           Generate comparison report with baseline"
            echo "  display           Display the benchmark report"
            echo "  save-baseline     Save current results as new baseline"
            echo "  health            Check webhook health"
            echo ""
            echo "Environment variables:"
            echo "  WEBHOOK_URL       Webhook server URL (default: http://localhost:8443)"
            echo "  VERSION           Version tag (default: git describe)"
            echo "  GIT_SHA           Git commit SHA (default: git rev-parse HEAD)"
            echo "  RUN_ID            Unique run identifier (default: local-timestamp)"
            exit 1
            ;;
    esac
}

main "$@"
