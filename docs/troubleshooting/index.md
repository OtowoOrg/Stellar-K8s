# Troubleshooting Guide

Production runbooks and diagnostic references for Stellar-K8s incidents.

> Tracking: Closes #997

---

## Core Guides

| Topic | File | Use During |
|---|---|---|
| Common issue catalog | [common-issues.md](common-issues.md) | General triage |
| Networking troubleshooting | [networking.md](networking.md) | Connectivity and latency faults |
| Decision trees | [diagnostic-decision-trees.md](diagnostic-decision-trees.md) | Critical failure branch analysis |
| Command runbooks | [operator-runbooks.md](operator-runbooks.md) | Incident execution steps |
| Metrics and alerting | [metrics-alerting.md](metrics-alerting.md) | Monitoring and escalation |

## Escalation Notes

> For suspected security compromise, follow [../security/incident-response-playbook.md](../security/incident-response-playbook.md) before restarting affected validator workloads.
Solutions to common problems and issues with the Stellar-K8s operator and nodes.

## Table of Contents

- [Operator Troubleshooting](operator-troubleshooting.md) - Diagnostic tools, stuck states, reconciliation loops, performance bottlenecks, recovery runbooks, and incident response.
- [Common Issues](common-issues.md) - A compilation of general issues and solutions (installation, deployment, runtime, etc.).
- [Networking Troubleshooting](networking.md) - Troubleshooting guide for P2P connection issues, network policies, mTLS, and CNI-specific issues.
- [Disk Scaling](disk-scaling.md) - Diagnosing dynamic volume expansion and resizing issues.
- [Sync Problems](sync-problems.md) - Troubleshooting catchup lag and SCP joining failures.
