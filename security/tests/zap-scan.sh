#!/bin/bash
# OWASP ZAP Baseline Penetration Test for Stellar-K8s Operator API
# Usage: ./zap-scan.sh http://localhost:9090

set -e

TARGET_URL=${1:-http://localhost:9090}
REPORT_DIR=security/reports
mkdir -p $REPORT_DIR

docker run -t --rm \\
  -v $REPORT_DIR:/reports \\
  -e TARGET=$TARGET_URL \\
  -e AUTOSEED_URL=$TARGET_URL \\
  --user root \\
  ghcr.io/zaproxy/zap-stable \\
  zap-baseline.py \\
    -t $TARGET_URL \\
    -r /reports/zap-baseline.html \\
    -w /reports/zap-baseline.xml \\
    --auto-seed \\
    -J /reports/zap-json-report.json

echo "✅ ZAP scan complete. Reports in $REPORT_DIR/"
echo "HTML: $REPORT_DIR/zap-baseline.html"

