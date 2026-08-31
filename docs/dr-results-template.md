# DR Drill Results Tracking

This document provides a template for tracking disaster recovery drill results and a monthly schedule for automated drills.

## Monthly Drill Schedule

| Month | Date | Drill Type | Target | RTO Target | RPO Target | Owner |
|-------|------|------------|--------|------------|------------|-------|
| January | 1st | Node Failure | Validator nodes | 5 min | 0 (streaming) | Platform |
| January | 15th | Network Partition | Inter-node communication | 10 min | 0 (streaming) | Platform |
| January | 28th | Storage Failure | Data persistence layer | 15 min | 0 (snapshots) | Platform |
| February | 1st | Node Failure | Validator nodes | 5 min | 0 (streaming) | Platform |
| February | 15th | Network Partition | Inter-node communication | 10 min | 0 (streaming) | Platform |
| February | 28th | Storage Failure | Data persistence layer | 15 min | 0 (snapshots) | Platform |
| March | 1st | Full DR Failover | Primary cluster | 30 min | 0 (streaming) | Platform |
| March | 15th | Node Failure | Validator nodes | 5 min | 0 (streaming) | Platform |
| March | 28th | Storage Failure | Data persistence layer | 15 min | 0 (snapshots) | Platform |

## Drill Execution Log Template

```markdown
# DR Drill Report — [DATE]

## Environment
- **Cluster:** [cluster-name]
- **Namespace:** [namespace]
- **Operator Version:** [version]
- **Drill Type:** [node-failure | network-partition | storage-failure]

## Pre-Drill State
- **Block Height:** [height]
- **Consensus Status:** [healthy | degraded]
- **All Validators Online:** [yes | no]
- **Monitoring Alerts:** [none | list]

## Drill Execution
- **Start Time:** [ISO-8601]
- **Fault Injected:** [description]
- **Duration:** [seconds]

## Recovery
- **Recovery Detected:** [ISO-8601]
- **Recovery Time:** [seconds]
- **RTO Target:** [seconds]
- **RTO Met:** [yes | no]

## Post-Drill State
- **Block Height:** [height]
- **Consensus Status:** [healthy | degraded]
- **All Validators Online:** [yes | no]
- **Data Integrity:** [verified | compromised]
- **Monitoring Alerts:** [none | list]

## Result
- **Status:** [PASS | FAIL]
- **Notes:** [observations]

## Corrective Actions (if FAIL)
- [ ] Action 1
- [ ] Action 2
```

## JSON Results Format

Automated drills produce structured JSON results stored in `/results/chaos/`:

```json
{
  "drill": "node-failure",
  "environment": "stellar-chaos",
  "start_time": "2026-01-01T02:00:00Z",
  "failure_injected": "pod-kill validator-0",
  "recovery_time_seconds": 120,
  "rto_target_seconds": 300,
  "rto_met": true,
  "result": "PASS",
  "notes": "Validator recovered within 2 minutes"
}
```

## RTO Targets by Drill Type

| Drill Type | RTO Target | Measurement Method |
|------------|------------|-------------------|
| Node Failure | 5 min (300s) | Time from pod kill to StellarNode Ready=True |
| Network Partition | 10 min (600s) | Time from network policy apply to consensus restoration |
| Storage Failure | 15 min (900s) | Time from volume unmount to data recovery |
| Full DR Failover | 30 min (1800s) | Time from primary failure to standby promotion |

## Escalation Procedure

1. **Drill Failure:** Stop drill, recover all faults immediately
2. **RTO Exceeded:** Notify on-call engineer within 30 minutes
3. **Data Integrity Issue:** Escalate to platform team lead within 1 hour
4. **Repeated Failures:** Escalate to engineering management within 2 hours

## Safety Boundaries

- All drills execute in the `stellar-chaos` namespace
- Drills are labeled with `dr-test=true` for identification
- Automatic cleanup occurs after drill completion
- Production clusters require explicit approval before drill execution
- Drills never target the primary production cluster without maintenance window
