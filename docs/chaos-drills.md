# Chaos Engineering Drills for Disaster Recovery

## Overview

This document outlines the monthly chaos engineering drill schedule, procedures, and results tracking for validating disaster recovery capabilities.

## Drill Schedule

| Month | Focus Area | Fault Type | Target | RTO Target |
|-------|------------|------------|--------|------------|
| January | Node Failure | Pod Kill | Validator nodes | 5 min |
| February | Network Partition | Network Latency/Loss | Inter-node communication | 10 min |
| March | Storage Failure | Disk Fill/Latency | Data persistence layer | 15 min |
| April | DNS Failure | DNS Resolution | Service discovery | 5 min |
| May | CPU/Memory Stress | Resource Exhaustion | Critical services | 10 min |
| June | Full DR Failover | Combined faults | Primary cluster | 30 min |
| July | Node Failure | Pod Kill | Validator nodes | 5 min |
| August | Network Partition | Network Latency/Loss | Inter-node communication | 10 min |
| September | Storage Failure | Disk Fill/Latency | Data persistence layer | 15 min |
| October | DNS Failure | DNS Resolution | Service discovery | 5 min |
| November | CPU/Memory Stress | Resource Exhaustion | Critical services | 10 min |
| December | Full DR Failover | Combined faults | Primary cluster | 30 min |

## Drill Procedures

### 1. Node Failure Drill

**Objective:** Verify system resilience when validator nodes fail unexpectedly.

**Pre-conditions:**
- Cluster is healthy and producing blocks
- All monitoring alerts are cleared
- Backup verification completed

**Procedure:**
1. Record current block height and consensus state
2. Execute node kill fault: `stellar-operator chaos run node-kill --duration 60s`
3. Monitor recovery time and block production
4. Verify consensus is restored within RTO (5 minutes)
5. Document any data loss or inconsistencies

**Success Criteria:**
- Block production resumes within 2 minutes
- Consensus restored within 5 minutes
- No data corruption detected
- All validators rejoin successfully

### 2. Network Partition Drill

**Objective:** Verify system behavior under network degradation.

**Pre-conditions:**
- Network monitoring active
- Baseline latency measurements recorded

**Procedure:**
1. Record baseline network metrics
2. Inject network latency (500ms) and packet loss (10%)
3. Monitor consensus and block production
4. Observe partition tolerance mechanisms
5. Remove fault and verify recovery

**Success Criteria:**
- Consensus maintained during partition
- Block production continues (may be slower)
- No split-brain conditions
- Full recovery within 10 minutes

### 3. Storage Failure Drill

**Objective:** Verify data persistence and recovery under storage stress.

**Pre-conditions:**
- Recent backup available
- Storage monitoring active

**Procedure:**
1. Record current state root hash
2. Inject disk fill (80% capacity) and increased latency
3. Monitor write operations and persistence
4. Verify checkpoint creation
5. Test recovery from backup if needed

**Success Criteria:**
- Critical data persisted successfully
- Recovery from backup within 15 minutes
- No data corruption
- State root hash integrity maintained

### 4. DNS Failure Drill

**Objective:** Verify service discovery resilience.

**Pre-conditions:**
- DNS monitoring active
- Service mesh health verified

**Procedure:**
1. Record DNS resolution times
2. Block DNS resolution for critical services
3. Monitor service discovery fallback mechanisms
4. Verify cached DNS entries used
5. Restore DNS and verify recovery

**Success Criteria:**
- Services continue operating with cached DNS
- No cascading failures
- Recovery within 5 minutes
- All services reconnected

### 5. Resource Exhaustion Drill

**Objective:** Verify system behavior under CPU/memory pressure.

**Pre-conditions:**
- Resource monitoring active
- Baseline resource usage recorded

**Procedure:**
1. Record baseline CPU/memory usage
2. Inject CPU load (90%) and memory pressure
3. Monitor OOM kills and throttling
4. Verify graceful degradation
5. Remove fault and verify recovery

**Success Criteria:**
- Critical services remain available
- OOM kills handled gracefully
- Recovery within 10 minutes
- No cascading failures

## Results Tracking

Detailed results tracking templates and JSON output format are documented in [dr-results-template.md](dr-results-template.md).

### Drill Execution Log

| Date | Drill Type | Duration | RTO Actual | Pass/Fail | Notes |
|------|------------|----------|------------|-----------|-------|
| | | | | | |

### Metrics to Track

- **Recovery Time Objective (RTO):** Time to restore normal operations
- **Recovery Point Objective (RPO):** Amount of data loss acceptable
- **Mean Time to Detection (MTTD):** Time to detect failure
- **Mean Time to Recovery (MTTR):** Average recovery time across drills

### Post-Drill Checklist

- [ ] All monitoring alerts cleared
- [ ] Block production resumed
- [ ] Consensus restored
- [ ] No data corruption detected
- [ ] All validators online
- [ ] Performance metrics normalized
- [ ] Incident report filed (if applicable)

## Automation

### Automated Drill Execution

```bash
# Run node failure drill
./scripts/run-chaos-drill.sh --type node-kill --duration 60s --target validator

# Run network partition drill
./scripts/run-chaos-drill.sh --type network --latency 500 --packet-loss 10

# Run storage failure drill
./scripts/run-chaos-drill.sh --type disk --fill-percent 80
```

### Scheduled Execution

Drills are scheduled monthly via Kubernetes CronJobs (`config/chaos-drills/cronjobs.yaml`):

- **1st of month (2 AM):** Node failure drill
- **15th of month (2 AM):** Network partition drill
- **28th of month (2 AM):** Storage failure drill

### Results Aggregation

After each scheduled window, aggregate the JSON drill artifacts into the tracked
summary used for the monthly review:

```bash
./scripts/aggregate-chaos-results.sh        # reads ./results/chaos/*.json
./scripts/aggregate-chaos-results.sh /data/drills   # or point at a results dir
```

The aggregator prints a chronological drill log (date, drill, RTO target, actual
RTO, pass/fail) and rewrites `results/chaos/RESULTS.md`. It exits non-zero when
any recorded drill missed its RTO target, which lets the monthly review gate on
a single command.

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: chaos-drill-monthly
  namespace: stellar-chaos
spec:
  schedule: "0 2 1 * *"  # 2 AM on 1st of every month
  jobTemplate:
    spec:
      template:
        spec:
          containers:
          - name: chaos-drill
            image: stellar-operator:latest
            command: ["/scripts/run-chaos-drill.sh"]
          restartPolicy: OnFailure
```

## Escalation Procedures

If a drill fails or reveals critical issues:

1. **Immediate:** Stop drill and recover all faults
2. **30 minutes:** Notify on-call engineer
2. **1 hour:** Escalate to platform team lead
3. **2 hours:** Escalate to engineering management

## References

- [Chaos Engineering Principles](https://principlesofchaos.org/)
- [Litmus Chaos Documentation](https://litmuschaos.io/docs/)
- [Stellar Network DR Plan](./dr-failover.md)
