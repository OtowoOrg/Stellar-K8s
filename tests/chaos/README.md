# Chaos Engineering Test Suite

Stellar-K8s ships a continuous chaos testing suite built on
[Chaos Mesh](https://chaos-mesh.org). It proves the operator survives
catastrophic cluster events and always converges `StellarNode` resources back
to a healthy state.

---

## Experiments

| # | File | Category | Severity | SLO |
|---|------|----------|----------|-----|
| 01 | `01-operator-pod-kill.yaml` | pod-failure | 🔴 Critical | 180s |
| 02 | `02-network-partition.yaml` | network | 🔴 Critical | 180s |
| 03 | `03-api-latency.yaml` | network | 🟠 High | 600s |
| 04 | `04-validator-peer-partition.yaml` | network | 🟠 High | 300s |
| 05 | `05-disk-fill.yaml` | storage | 🟡 Medium | 120s |
| 06 | `06-cpu-stress.yaml` | resource-exhaustion | 🟡 Medium | 300s |
| 07 | `07-memory-pressure.yaml` | resource-exhaustion | 🟡 Medium | 300s |
| 08 | `08-validator-pod-kill.yaml` | pod-failure | 🟠 High | 300s |
| 09 | `09-cascading-failure.yaml` | cascading | 🔴 Critical | 600s |
| 10 | `10-io-stress.yaml` | storage | 🟡 Medium | 300s |

---

## What each experiment verifies

### 01 — Operator Pod Kill
- **Chaos:** SIGKILL on the operator pod every 5s for 30s
- **Verifies:** Kubernetes restarts the operator; it re-reconciles all
  StellarNodes from scratch with no human intervention

### 02 — Network Partition
- **Chaos:** Full bidirectional TCP block between operator and K8s API for 60s
- **Verifies:** Operator handles `connection refused` gracefully; resumes after
  partition heals with no corrupt state

### 03 — API High Latency
- **Chaos:** 2000ms delay + 500ms jitter on every API call for 120s
- **Verifies:** Operator does not time out fatally, does not create duplicate
  resources, and eventually converges despite slow API

### 04 — Validator Peer Partition
- **Chaos:** Validator pods partitioned from each other (SCP peer ports) for 90s
- **Verifies:** Operator detects partition, updates StellarNode status, and
  recovers consensus after partition heals

### 05 — Disk Fill
- **Chaos:** Operator pod disk filled to capacity for 45s
- **Verifies:** Operator handles disk-full errors gracefully; does not crash or
  corrupt cluster state

### 06 — CPU Stress
- **Chaos:** 4 CPU workers at 90% load on the operator pod for 60s
- **Verifies:** Reconciliation loop continues under CPU pressure; no deadlocks

### 07 — Memory Pressure
- **Chaos:** 256 MB memory consumed on the operator pod for 60s
- **Verifies:** Operator is not OOM-killed; reconciliation continues

### 08 — Validator Pod Kill
- **Chaos:** Validator pods killed every 15s for 45s
- **Verifies:** Operator detects pod deaths, triggers StatefulSet re-creation,
  and updates StellarNode status correctly

### 09 — Cascading Failure
- **Chaos:** Simultaneous pod kill + network partition (worst-case scenario)
- **Verifies:** Operator restarts and reconnects; no split-brain or duplicate
  resource creation

### 10 — I/O Stress
- **Chaos:** Validator pod storage saturated with 4 concurrent I/O workers for 60s
- **Verifies:** Operator detects I/O degradation; does NOT delete the PVC;
  StellarNode recovers after stress ends

---

## Running locally

You need only **Docker** installed. The script installs everything else.

```bash
# From the project root:
chmod +x tests/chaos/run-chaos-tests.sh
./tests/chaos/run-chaos-tests.sh
```

The script will:
1. Check for `docker`, `kubectl`, `helm`, `kind`, `python3`
2. Create a 3-node kind cluster called `stellar-chaos`
3. Install Chaos Mesh and deploy the operator
4. Run all 10 experiments in sequence
5. Generate a resilience report in `tests/chaos/results/<run-id>/`

### Options

```bash
# Skip cluster creation (cluster already running)
SKIP_SETUP=true ./tests/chaos/run-chaos-tests.sh

# Run only specific experiments
EXPERIMENTS="01 02 09" ./tests/chaos/run-chaos-tests.sh

# Skip specific experiments (e.g. skip cascading failure)
SKIP_EXPERIMENTS="09" ./tests/chaos/run-chaos-tests.sh

# Custom recovery timeout
RECOVERY_TIMEOUT=600 ./tests/chaos/run-chaos-tests.sh
```

### Running a single experiment manually

```bash
# Apply the experiment
kubectl apply -f tests/chaos/01-operator-pod-kill.yaml -n chaos-testing

# Wait for it to finish
sleep 40

# Remove it
kubectl delete -f tests/chaos/01-operator-pod-kill.yaml -n chaos-testing

# Check operator recovered
kubectl get pods -n stellar-system
kubectl get stellarnode --all-namespaces
```

### Cleanup

```bash
kind delete cluster --name stellar-chaos
```

---

## Running in CI (GitHub Actions)

The workflow lives at `.github/workflows/chaos-tests.yml`.

**Triggers:**
- **Nightly** — every day at 02:00 UTC automatically
- **Manual** — Actions → Chaos Engineering Tests → Run workflow

**Parallel execution:** Experiments run in 3 parallel groups on separate kind
clusters to keep total CI time under 90 minutes:

| Group | Experiments |
|-------|-------------|
| A | 01 Pod Kill, 02 Network Partition |
| B | 03 API Latency, 04 Peer Partition, 05 Disk Fill |
| C | 06 CPU Stress, 07 Memory Pressure, 08 Validator Kill, 09 Cascading, 10 I/O |

**Skipping experiments:** When triggering manually, enter a comma-separated list
of experiment IDs in the "skip_experiments" input (e.g. `"09,10"`).

---

## Resilience Report

After each run, `generate_report.py` produces:

- `resilience-report.md` — human-readable Markdown with per-experiment scores
- `resilience-report.json` — machine-readable JSON for trend tracking

### Scoring rubric

| Condition | Score impact |
|-----------|-------------|
| System recovered within SLO | 100 pts |
| Recovered but SLO breached | 70–99 pts (proportional to overshoot) |
| System did not recover | 0 pts |

**Weighted overall score:** Critical experiments count 3×, High 2×, Medium 1×.

### Score interpretation

| Score | Verdict |
|-------|---------|
| 90–100 | 🟢 Excellent |
| 70–89 | 🟡 Good |
| 50–69 | 🟠 Fair |
| 0–49 | 🔴 Poor |

---

## Interpreting logs

**Healthy recovery after pod kill:**
```
WARN reconciliation error ... connection refused
INFO Reconciling StellarNode stellar-system/chaos-test-horizon
INFO Applied StellarNode: stellar-system/chaos-test-horizon
```

**Healthy recovery after network partition:**
```
WARN KubeError: ... timeout
WARN KubeError: ... connection refused
INFO Reconciled: ObjectRef { name: "chaos-test-horizon" ... }
```

**Red flags (experiment failed):**
- Operator pod stays in `CrashLoopBackOff` after chaos ends
- StellarNode stuck in `Failed` phase after the recovery timeout
- Duplicate Deployments or Services created
- Finalizers stuck preventing StellarNode deletion
- `panic` in operator logs
