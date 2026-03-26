# PrometheusRule Alerts Implementation
## Status: In Progress

Steps:
- [x] 1. Checkout blackboxai/prometheus-alerts-rules from main
- [x] 2. Add prometheusAlerts to charts/stellar-operator/values.yaml
- [x] 3. Create charts/stellar-operator/templates/prometheusrule.yaml with 4 alerts
- [x] 4. Test helm template charts/stellar-operator (generated PrometheusRule)
- [x] 5. Validate manifests (helm lint pass, cargo test pass)
- [ ] 6. Commit, push, PR to main

Alerts:
1. StellarNodeSyncLag: stellar_core_ledger_age > 100 or stellar_node_ingestion_lag > 100
2. StellarNodeMemoryPressure: container_memory_working_set_bytes / limit > 0.9
3. StellarOperatorReconcileErrors: rate(stellar_reconcile_errors_total[5m]) > 0
4. StellarHistoryArchiveUnresponsive: (probe for history URLs or error rate)
