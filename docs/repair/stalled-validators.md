# Automated Node Repair for Stalled Validators

## Overview
Self-healing component that detects stalled validator nodes and systematically attempts tiered remediation without manual intervention.

## Stalled Criteria
- A Node is identified as stalled if it fails to close a ledger for more than 5 minutes.
- This is evaluated via the `stellar_core_ledger_close_time` Prometheus metrics.

## Tiered Remediation Logic
1. **Tier 1 (Soft Restart)**: Gracefully restart the `stellar-core` container.
2. **Tier 2 (DB Rebuild)**: Trigger a database rebuild from historical archives.
3. **Tier 3 (Pod Reschedule)**: Evict and reschedule the Pod on a new underlying node.

## Safety Measures
- Implements safety backoffs to avoid thrashing.
- Halts repair operations if a global or network-wide partition is detected (e.g., when the majority of nodes are stalled).
- Triggers operational alerts on every repair action taken.
