# Performance Profiling Runbook

**Issue:** #1386 — Add performance profiling integration for Rust services  
**Audience:** SREs, operators, platform engineers

---

## Overview

Stellar-K8s exposes pprof-compatible CPU and heap profiling endpoints
gated behind a shared secret token. Profiling is **disabled by default**
and only activates when the `profiling` Helm value is enabled and the
image is built with `--features profiling`.

---

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| `kubectl` | 1.28+ | Port-forwarding |
| `go tool pprof` | any | Profile analysis |
| `curl` | any | Capture profiles |

---

## Enabling Profiling

### 1 — Generate a token and its SHA-256 hash

```bash
TOKEN=$(openssl rand -hex 32)
echo "Token: $TOKEN"
echo -n "$TOKEN" | sha256sum | awk '{print $1}'
# → store that hex digest in values.yaml
```

### 2 — Create the Kubernetes Secret

```bash
kubectl create secret generic stellar-profiling-token \
  --from-literal=token="$TOKEN" \
  -n stellar-system
```

### 3 — Enable profiling in Helm values

```yaml
# values-production.yaml
profiling:
  enabled: true
  bindAddr: "127.0.0.1:6060"
  tokenSecretName: "stellar-profiling-token"
  tokenSha256: "<hex digest from step 1>"
  defaultCpuDurationSecs: 30
  maxCpuDurationSecs: 300
```

Deploy:

```bash
helm upgrade stellar-operator ./charts/stellar-operator \
  -n stellar-system \
  -f values-production.yaml
```

The operator image must be built with:

```bash
cargo build --release --features profiling
```

> **Security note:** The profiling server binds to `127.0.0.1` only and is
> never reachable via the cluster `Service` or ingress. All access requires
> `kubectl port-forward`.

---

## Capturing Profiles

### Port-forward the operator pod

```bash
POD=$(kubectl get pods -n stellar-system \
  -l app=stellar-operator -o jsonpath='{.items[0].metadata.name}')

kubectl port-forward "$POD" 6060:6060 -n stellar-system &
```

### Retrieve the token

```bash
TOKEN=$(kubectl get secret stellar-profiling-token \
  -n stellar-system \
  -o jsonpath='{.data.token}' | base64 -d)
```

### CPU profile (30-second sample)

```bash
curl -sSf \
  -H "X-Profiling-Token: $TOKEN" \
  "http://localhost:6060/debug/pprof/profile?duration=30" \
  -o cpu.pb.gz

go tool pprof cpu.pb.gz
```

### Heap profile

```bash
curl -sSf \
  -H "X-Profiling-Token: $TOKEN" \
  "http://localhost:6060/debug/pprof/heap" \
  -o heap.pb.gz

go tool pprof heap.pb.gz
```

### Memory allocation profile

```bash
curl -sSf \
  -H "X-Profiling-Token: $TOKEN" \
  "http://localhost:6060/debug/pprof/allocs" \
  -o allocs.pb.gz

go tool pprof allocs.pb.gz
```

### Profile index

```bash
curl -sSf -H "X-Profiling-Token: $TOKEN" \
  http://localhost:6060/debug/pprof/
```

---

## Analysing CPU Profiles

```bash
# Interactive terminal UI
go tool pprof cpu.pb.gz

# Top 20 functions by cumulative time
(pprof) top 20 -cum

# Flame graph (requires Graphviz)
(pprof) web

# Export as SVG
go tool pprof -svg cpu.pb.gz > flame.svg

# Identify reconciler hot paths
(pprof) list reconcile
```

### Common bottleneck patterns

| Symptom | Likely cause | Investigation |
|---------|-------------|---------------|
| High `reconcile` CPU | Large StellarNode fleet + short requeue interval | `(pprof) list apply_stellar_node` |
| High `kube::api` time | Kubernetes API latency / too many LIST calls | Check audit log for LIST frequency |
| High `serde_json` time | Excess JSON (de)serialization in status patches | Profile `update_status` paths |
| High `reqwest` time | Slow Horizon / Stellar Core health checks | Increase `healthCheckTimeout` |

---

## Analysing Heap Profiles

```bash
# Show top memory allocations
go tool pprof heap.pb.gz
(pprof) top 20 -cum

# Find memory leaks (compare two snapshots)
go tool pprof -diff_base heap_before.pb.gz heap_after.pb.gz
(pprof) top 10 -cum
```

---

## Disabling Profiling

```yaml
profiling:
  enabled: false
```

```bash
helm upgrade stellar-operator ./charts/stellar-operator \
  -n stellar-system \
  -f values-production.yaml
```

The profiling server stops accepting connections immediately after the
pod restarts.

---

## Security Considerations

1. **Token rotation** — rotate `stellar-profiling-token` quarterly or
   after suspected compromise.  Update `tokenSha256` in values and
   redeploy.
2. **Never expose port 6060** via a `Service` or ingress — always use
   `kubectl port-forward`.
3. **Least-privilege access** — only operators who can `kubectl exec`
   into the pod should know the token.
4. **Audit log** — every successful profile capture is logged at `INFO`
   level with the profile type and duration.

---

## Troubleshooting

| Error | Cause | Fix |
|-------|-------|-----|
| `403 Forbidden` | Wrong or missing token | Verify `X-Profiling-Token` header |
| `501 Not Implemented` | Image not built with `--features profiling` | Rebuild / use profiling-enabled image |
| `connection refused` | Port-forward not running | Re-run `kubectl port-forward` |
| Heap profile empty | `MALLOC_CONF` not set | Ensure jemalloc `prof:true` is configured |

---

## Reference

- [`src/profiling.rs`](../src/profiling.rs) — Core profiling implementation
- [pprof format spec](https://github.com/google/pprof/blob/main/proto/profile.proto)
- [jemalloc heap profiling](https://jemalloc.net/jemalloc.3.html#opt.prof)
- [Helm values reference](../charts/stellar-operator/values.yaml) — `profiling.*`
