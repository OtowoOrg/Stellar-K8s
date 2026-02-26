# Chaos Engineering Tests

This directory contains the Stellar-K8s chaos engineering test suite, built on
[Chaos Mesh](https://chaos-mesh.org). It proves the operator survives
catastrophic cluster events and always converges `StellarNode` resources back
to a healthy state.

---

## What's in here

| File | What it does |
|------|-------------|
| `01-operator-pod-kill.yaml` | Kills the operator pod while it's reconciling |
| `02-network-partition.yaml` | Cuts all network between the operator and the K8s API |
| `03-api-latency.yaml` | Adds 2s latency to every API call the operator makes |
| `run-chaos-tests.sh` | Runs all three experiments end-to-end on a local kind cluster |

---

## Running locally (from zero)

You need only **Docker** installed. The script installs everything else.

```bash
# From the project root:
chmod +x tests/chaos/run-chaos-tests.sh
./tests/chaos/run-chaos-tests.sh
```

That's it. The script will:
1. Install `kind`, `kubectl`, and `helm` if they're missing
2. Create a 3-node kind cluster called `stellar-chaos`
3. Install Chaos Mesh into it
4. Build and deploy the operator
5. Run all 3 experiments in sequence
6. Print PASS/FAIL for each and save logs to `tests/chaos/results/`

### Re-running without recreating the cluster

If you already have the cluster running and just want to re-run the experiments:

```bash
SKIP_SETUP=true ./tests/chaos/run-chaos-tests.sh
```

### Running a single experiment manually

```bash
# Apply the experiment
kubectl apply -f tests/chaos/01-operator-pod-kill.yaml -n chaos-testing

# Wait for it to finish (check duration in the YAML)
sleep 40

# Remove it
kubectl delete -f tests/chaos/01-operator-pod-kill.yaml -n chaos-testing

# Check operator recovered
kubectl get pods -n stellar-system
kubectl get stellarnode --all-namespaces
```

### Cleaning up

```bash
kind delete cluster --name stellar-chaos
```

---

## Running in CI (GitHub Actions)

The workflow lives at `.github/workflows/chaos-tests.yml`.

It runs:
- **Manually** — go to Actions → Chaos Engineering Tests → Run workflow
- **Weekly** — every Sunday at midnight UTC automatically

To trigger it manually on any branch:
1. Go to your repo on GitHub
2. Click **Actions** tab
3. Click **Chaos Engineering Tests** in the left sidebar
4. Click **Run workflow** → choose your branch → click the green button

You can also skip a specific experiment by entering `01`, `02`, or `03` in the
"Skip a specific experiment" input when triggering manually.

---

## What each experiment verifies

### Experiment 1 — Operator Pod Kill
- **Chaos:** The operator pod receives SIGKILL (immediate, no graceful shutdown)
- **Duration:** 30 seconds of repeated kills
- **Verifies:** Kubernetes restarts the operator via the Deployment; the operator
  re-reconciles all StellarNodes from scratch with no human intervention

### Experiment 2 — Network Partition
- **Chaos:** All TCP traffic between the operator and the K8s API is blocked
- **Duration:** 60 seconds
- **Verifies:** The operator handles `connection refused` errors gracefully
  (no panics, no corrupt state); once the partition heals it resumes normally

### Experiment 3 — API High Latency
- **Chaos:** Every packet from the operator gets a 2000ms delay + 500ms jitter
- **Duration:** 120 seconds
- **Verifies:** The operator does not time out fatally, does not create duplicate
  resources, and eventually converges despite slow API responses

---

## Interpreting results

Logs for each experiment are saved to `tests/chaos/results/<timestamp>/`.

**What healthy recovery looks like in the logs:**

For pod kill you should see lines like:
```
WARN reconciliation error ... connection refused
INFO Reconciling StellarNode stellar-system/chaos-test-horizon
INFO Applying StellarNode: stellar-system/chaos-test-horizon
```

For network partition/latency you should see retries followed by eventual success:
```
WARN KubeError: ... timeout
INFO Reconciled: ObjectRef { name: "chaos-test-horizon" ... }
```

**Red flags (experiment failed):**
- Operator pod stays in `CrashLoopBackOff` after chaos ends
- StellarNode stuck in `Failed` phase after the recovery timeout
- Duplicate Deployments or Services created
- Finalizers stuck preventing StellarNode deletion