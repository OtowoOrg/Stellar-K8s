# kube-bench compliance (CIS)

This directory integrates [kube-bench](https://github.com/aquasecurity/kube-bench)
into Stellar-K8s (issue #1380). kube-bench checks a cluster against the CIS
Kubernetes Benchmark; the custom control set here focuses on control-plane
components and kubelets that run operator-managed workloads.

## Layout

| Path | Purpose |
|---|---|
| `config/stellar-bench.yaml` | Custom kube-bench **controls** (`stellar-bench` benchmark) |
| `security/kube-bench/rbac.yaml` | Namespace + minimal read-only RBAC for the scan job |
| `security/kube-bench/job.yaml` | In-cluster Job that runs the scan on the control-plane node |
| `security/kube-bench/report-parser.py` | Renders a summary from kube-bench JSON output |
| `security/kube-bench/run-local.sh` | Static check / best-effort in-cluster helper |
| `security/kube-bench/README.md` | This file |

## CI

`.github/workflows/compliance-scan.yml` runs on every PR. It boots a kind
control-plane node, runs `kube-bench run --benchmark stellar-bench`, posts a
summary to the workflow run, and attaches the raw JSON report. The job is
**non-blocking**: scan findings never fail the PR, and the custom-policy parse
is guarded so tooling drift cannot break CI.

## Running manually

```bash
make compliance-test                 # static validation (no cluster required)
bash security/kube-bench/run-local.sh
```

For a real scan against a cluster:

```bash
kubectl apply -f security/kube-bench/rbac.yaml
kubectl create configmap stellar-bench-controls -n kube-bench \
  --from-file=stellar-bench.yaml=config/stellar-bench.yaml \
  --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f security/kube-bench/job.yaml
kubectl -n kube-bench logs job/kube-bench > kube-bench-report.json
python3 security/kube-bench/report-parser.py kube-bench-report.json
```

Local one-liner:

```bash
docker run --rm --pid=host -v /etc:/etc:ro \
  -v /opt/kube-bench/cfg/stellar-bench.yaml:/opt/kube-bench/cfg/stellar-bench.yaml \
  aquasec/kube-bench:v0.9.5 run --benchmark stellar-bench --output json
```

## Custom benchmark

`config/stellar-bench.yaml` is selected with `--benchmark stellar-bench` and is
kept as a small, reviewable set: detailed hard-fail checks (anonymous-auth,
read-only-port, file ownership) plus documented manual checks for configuration
standards. Extend it the same way the CIS control files do; each check needs
`id`, `text`, and for automated checks `audit` + `tests`.

## Report parsing

`report-parser.py` tolerates kube-bench JSON structure and prints a markdown
summary (used for the GitHub job summary) with per-check PASS/FAIL/WARN/INFO
and totals. It always exits 0 so reporting never blocks a merge.