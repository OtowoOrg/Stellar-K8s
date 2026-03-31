#!/usr/bin/env bash
#
# Quick local test for webhook benchmarks
# This script starts the webhook server and runs a quick benchmark

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

# Cleanup function
cleanup() {
    if [ -n "${WEBHOOK_PID:-}" ]; then
        log_info "Stopping webhook server (PID: $WEBHOOK_PID)..."
        kill $WEBHOOK_PID 2>/dev/null || true
        wait $WEBHOOK_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

# Build webhook server
log_info "Building webhook server..."
cd "$PROJECT_ROOT"
cargo build --release --quiet

# Start webhook server
log_info "Starting webhook server..."
./target/release/stellar-operator webhook --bind 0.0.0.0:8443 > /tmp/webhook.log 2>&1 &
WEBHOOK_PID=$!

# Wait for webhook to be ready
log_info "Waiting for webhook to be ready..."
for i in {1..30}; do
    if curl -sf http://localhost:8443/health > /dev/null 2>&1; then
        log_success "Webhook is ready!"
        break
    fi
    if [ $i -eq 30 ]; then
        log_error "Webhook failed to start within 30 seconds"
        cat /tmp/webhook.log
        exit 1
    fi
    sleep 1
done

# Run quick benchmark (reduced duration for local testing)
log_info "Running quick benchmark..."

k6 run \
    --env WEBHOOK_URL=http://localhost:8443 \
    --env VERSION=local-test \
    --env GIT_SHA=local \
    --env RUN_ID=test-$(date +%s) \
    --duration 30s \
    --vus 10 \
    "${SCRIPT_DIR}/k6/webhook-load-test.js" || true

log_success "Benchmark completed!"

# Display results if available
if [ -f results/webhook-benchmark-report.md ]; then
    echo ""
    echo "==================================================================="
    cat results/webhook-benchmark-report.md
    echo "==================================================================="
fi
