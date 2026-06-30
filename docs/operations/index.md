# Operations Runbook

This runbook guides operators through day-to-day maintenance, incident response, and disaster recovery for Stellar-K8s.

## Daily Operations

- Monitor operator health and reconciliation throughput.
- Verify that `StellarNode` resources are ready.
- Review `kubectl get events` for transient failures.
- Confirm backups and snapshots are completing on schedule.

## Monitoring and Alerts

Key signals to monitor:
- Operator liveness and readiness (`/livez`, `/readyz`)
- Reconciliation error rate
- Node pod crash loops
- Admission webhook failures
- Prometheus scrape errors for managed workloads

## Capacity Planning

- Track the number of active `StellarNode` resources.
- Estimate control-plane load from reconciliation frequency.
- Plan horizontal scaling of the operator and webhook workloads as the cluster grows.

## Backup and Disaster Recovery

- Store operator manifests and CRD definitions in version control.
- Backup persistent Stellar Core storage and any attached volumes.
- Use Kubernetes snapshots or storage-provider backups for data recovery.
- Test restore workflows in a staging environment.

## Troubleshooting Common Issues

### `StellarNode` stuck in pending
- Confirm CRD validation passed.
- Check operator logs for reconcile failures.
- Validate pod resource requests and cluster capacity.

### Webhook errors
- Inspect the webhook pod logs for admission request failures.
- Confirm the webhook service endpoint is reachable by the API server.
- Validate TLS certificates if using a secure webhook.

### Operator restart loops
- Review operator config and health probe settings.
- Check for CRD validation failures or invalid cluster state.
- Ensure leader election and RBAC permissions are configured correctly.

## Related Documentation

- [Incident Response](incident-response.md)
- [API Reference](../api/index.md)
- [Production Security Hardening](../production-security-hardening.md)
