#!/bin/bash
# Example: Multi-cluster performance comparison scenarios

set -e

echo "=== Stellar-K8s Benchmark Compare Examples ==="
echo

# Example 1: Basic comparison between two contexts
echo "Example 1: Basic Comparison"
echo "----------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context prod-us-east \
  --cluster-b-context prod-us-west \
  --duration 60

echo
echo "Press Enter to continue to next example..."
read

# Example 2: A/B testing with custom labels
echo "Example 2: A/B Testing with Custom Labels"
echo "------------------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --cluster-a-label "Production (Current Config)" \
  --cluster-b-label "Staging (New Config)" \
  --duration 120 \
  --interval 10

echo
echo "Press Enter to continue to next example..."
read

# Example 3: Cloud provider comparison with HTML export
echo "Example 3: Cloud Provider Comparison"
echo "-------------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context aws-us-east-1 \
  --cluster-b-context gcp-us-central1 \
  --cluster-a-label "AWS (m5.2xlarge)" \
  --cluster-b-label "GCP (n2-standard-8)" \
  --duration 300 \
  --output cloud-comparison.html \
  --format html

echo "Report saved to cloud-comparison.html"
echo
echo "Press Enter to continue to next example..."
read

# Example 4: Using Prometheus URLs directly
echo "Example 4: Direct Prometheus URLs"
echo "----------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-prometheus http://prometheus-a.monitoring:9090 \
  --cluster-b-prometheus http://prometheus-b.monitoring:9090 \
  --cluster-a-label "Cluster A" \
  --cluster-b-label "Cluster B" \
  --duration 60

echo
echo "Press Enter to continue to next example..."
read

# Example 5: Extended duration with JSON export
echo "Example 5: Extended Duration with JSON Export"
echo "----------------------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --duration 600 \
  --interval 15 \
  --output performance-data.json \
  --format json

echo "Data saved to performance-data.json"
echo
echo "Analyzing results with jq..."
echo "Cluster A TPS (mean):"
jq '.cluster_a_summary.tps.mean' performance-data.json
echo "Cluster B TPS (mean):"
jq '.cluster_b_summary.tps.mean' performance-data.json

echo
echo "Press Enter to continue to next example..."
read

# Example 6: Hardware upgrade validation
echo "Example 6: Hardware Upgrade Validation"
echo "---------------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context old-hardware \
  --cluster-b-context new-hardware \
  --cluster-a-label "Old (m5.large, 2 vCPU, 8GB)" \
  --cluster-b-label "New (m5.xlarge, 4 vCPU, 16GB)" \
  --duration 300 \
  --output hardware-upgrade.html \
  --format html

echo "Report saved to hardware-upgrade.html"
echo
echo "Press Enter to continue to next example..."
read

# Example 7: Network optimization testing
echo "Example 7: Network Optimization Testing"
echo "----------------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context before-optimization \
  --cluster-b-context after-optimization \
  --cluster-a-label "Before Network Optimization" \
  --cluster-b-label "After Network Optimization" \
  --metrics tps,consensus_latency \
  --duration 180

echo
echo "Press Enter to continue to next example..."
read

# Example 8: Multi-region performance
echo "Example 8: Multi-Region Performance"
echo "------------------------------------"
stellar-operator benchmark-compare \
  --cluster-a-context us-east-1 \
  --cluster-b-context eu-west-1 \
  --cluster-a-label "US East (Virginia)" \
  --cluster-b-label "EU West (Ireland)" \
  --duration 300 \
  --output multi-region.html \
  --format html

echo "Report saved to multi-region.html"
echo
echo "Press Enter to continue to next example..."
read

# Example 9: Continuous monitoring (loop)
echo "Example 9: Continuous Monitoring"
echo "---------------------------------"
echo "Running 3 iterations with 60-second intervals..."

for i in {1..3}; do
  echo "Iteration $i/3"
  stellar-operator benchmark-compare \
    --cluster-a-context prod \
    --cluster-b-context staging \
    --duration 60 \
    --output "report-iteration-$i.html" \
    --format html
  
  if [ $i -lt 3 ]; then
    echo "Waiting 60 seconds before next iteration..."
    sleep 60
  fi
done

echo "All iterations complete. Reports saved as report-iteration-*.html"
echo
echo "Press Enter to continue to next example..."
read

# Example 10: Automated decision making
echo "Example 10: Automated Decision Making"
echo "--------------------------------------"
echo "Running comparison and making automated decision..."

stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --duration 120 \
  --output decision-data.json \
  --format json

# Parse results and make decision
PROD_TPS=$(jq '.cluster_a_summary.tps.mean' decision-data.json)
STAGING_TPS=$(jq '.cluster_b_summary.tps.mean' decision-data.json)

echo "Production TPS: $PROD_TPS"
echo "Staging TPS: $STAGING_TPS"

# Calculate percentage difference
DIFF=$(echo "scale=2; (($STAGING_TPS - $PROD_TPS) / $PROD_TPS) * 100" | bc)

echo "Difference: $DIFF%"

if (( $(echo "$DIFF > 5" | bc -l) )); then
  echo "✅ DECISION: Staging performs >5% better. Recommend promotion to production."
elif (( $(echo "$DIFF < -5" | bc -l) )); then
  echo "❌ DECISION: Staging performs >5% worse. Do NOT promote to production."
else
  echo "⚠️  DECISION: Performance difference is within 5%. Further testing recommended."
fi

echo
echo "=== All Examples Complete ==="
echo
echo "Generated files:"
ls -lh *.html *.json 2>/dev/null || echo "No files generated"
