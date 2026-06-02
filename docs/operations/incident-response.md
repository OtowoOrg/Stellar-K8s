# Incident Response Procedures

This page documents incident response procedures for Stellar-K8s.

## Severity Levels

- **P1** — Operator is down, reconciliation is failing, or cluster control plane is unavailable.
- **P2** — One or more `StellarNode` resources are degraded but the cluster remains partially functional.
- **P3** — Documentation, reporting, or non-urgent configuration changes.

## Response Workflow

1. Identify the affected workload and severity.
2. Triage logs from the operator and webhook pods.
3. Verify cluster health and `StellarNode` status conditions.
4. Escalate to on-call team if the issue impacts production availability.

## Common Incident Scenarios

### Operator Reconciliation Failure
- Review `stellar-operator` logs.
- Inspect CRD events and failed status conditions.
- Check for invalid `StellarNode` manifests or admission webhook denials.

### Admission Webhook Failure
- Confirm the webhook service is running and healthy.
- Validate certificate trust between API server and webhook.
- Examine the webhook response payload for rejected requests.

### Backup or Restore Failure
- Review snapshot logs and storage provider errors.
- Confirm data consistency before retrying.
- Execute restore testing in a non-production cluster first.

## Post-Mortem and Follow-up

- Document the root cause and recovery steps.
- Update the runbook with new mitigation patterns.
- Review related docs and training materials.
- Conduct a retro with stakeholders after major incidents.
