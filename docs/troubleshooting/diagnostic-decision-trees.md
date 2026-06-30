# Diagnostic Decision Trees

> Text-based flowcharts for high-severity Stellar-K8s incidents.

---

## 1) Operator Stuck Reconciliation Loop

```text
START
  |
  |-- Check metric: stellar_k8s_operator_reconciliation_duration_seconds (p95)
  |      |
  |      |-- p95 > 5s?
  |             |
  |             |-- YES --> Check operator logs for repeated conflict/finalizer errors
  |             |            |
  |             |            |-- "the object has been modified"?
  |             |            |      |
  |             |            |      |-- YES --> Verify optimistic lock retry + backoff config
  |             |            |      |-- NO  --> Check dependent API latency and webhook health
  |             |
  |             |-- NO --> Check workqueue depth and pending events
  |
  |-- Is the same CR reconciling repeatedly without status progression?
         |
         |-- YES --> Inspect finalizers, ownerRefs, blocked dependent resources
         |-- NO  --> Investigate intermittent cluster/API instability
```

---

## 2) Split-Brain Node Outage Risk

```text
START
  |
  |-- More than one validator pod reports active signer?
         |
         |-- YES --> SEV-1
         |          1) Remove both from service routing
         |          2) Inspect Lease holderIdentity + renewTime
         |          3) Preserve forensic data before restart
         |
         |-- NO --> Check intermittent lock handoff failures
```

---

## 3) Out-of-Sync Ledger Lag

```text
START
  |
  |-- Metric: stellar_core_ledger_age > 20s?
         |
         |-- YES --> Check network drops and peer reachability on 11625/tcp
         |           |
         |           |-- Packet drops elevated?
         |           |      |
         |           |      |-- YES --> Inspect CNI datapath (calicoctl/cilium monitor)
         |           |      |-- NO  --> Check disk IO and database contention
         |
         |-- NO --> Check application-level query latency if issue observed in RPC only
```

---

## 4) Missing Secret Access Errors

```text
START
  |
  |-- Pod events show "Forbidden" or mount failure for secret?
         |
         |-- YES --> Validate serviceAccount + RoleBinding namespace alignment
         |           |
         |           |-- Can service account get secret?
         |                  |
         |                  |-- NO --> Fix Role/RoleBinding scope
         |                  |-- YES --> Check secret name/key, rotation timestamp, CSI provider health
         |
         |-- NO --> Check application startup path and environment variable references
```
