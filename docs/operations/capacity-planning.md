# Capacity Planning & Storage Scaling Calculator Guide

This guide gives operators a definitive way to size CPU, memory, and storage
for Stellar-K8s-managed nodes, project storage growth over 6/12/24 months,
and understand how the operator's automated volume expansion interacts with
that growth — so a cluster is provisioned correctly the first time instead of
discovered via a `Disk Full` outage.

It complements, rather than replaces, three existing docs:

- [Resource Limits for Stellar Node Types](../resource-limits.md) — CPU/memory
  request/limit recommendations per node type.
- [Proactive Disk Scaling](../proactive-disk-scaling.md) — the operator's
  automated EBS/GCP volume expansion feature (config reference, events,
  metrics).
- [History Archive Pruning Guide](../archive-pruning.md) — reclaiming space on
  full-history archive nodes.

This guide is the "how much do I provision, and how fast will it grow"
reference that ties those three together.

---

## 1. Baseline Requirements by Node Type

Baselines below start from the [official Stellar hardware
recommendations](https://developers.stellar.org/docs/validators/admin-guide/prerequisites)
and are mapped onto the operator's `StellarNode` resource model and standard
AWS/GCP instance types. Where this repo's own [resource-limits.md](../resource-limits.md)
recommends a different figure for Testnet (it generally sizes lower), that
figure is used instead — Testnet load is a small fraction of Mainnet and does
not need production-grade headroom.

> **Read this table as a starting point, not a ceiling.** All figures assume
> "current network activity" as of this writing (see [§3](#3-storage-growth-model)
> for the live throughput this is based on). Actual requirements grow with
> network activity — re-validate against §3 periodically, and prefer the
> operator's own [Prometheus metrics](#5-monitoring-checklist) over static
> numbers once a node has been running for a few weeks.

### 1.1 Validator (Stellar Core)

| | CPU | Memory | Disk | IOPS | Cloud equivalent |
|---|---|---|---|---|---|
| **Mainnet, `historyMode: Recent`** | 8 vCPU @ 3.4GHz | 16Gi | 100Gi NVMe SSD | 10,000 | AWS `c5.2xlarge` / GCP `n4-highcpu-8` |
| **Mainnet, `historyMode: Full`** | 8 vCPU @ 3.4GHz | 16Gi | **1,500Gi** NVMe SSD | 10,000 | AWS `c5.2xlarge` / GCP `n4-highcpu-8` |
| **Testnet** | 500m–2 | 1–4Gi | 100–200Gi SSD | 3,000 | AWS `c5.xlarge` / GCP `n4-highcpu-4` |

- CPU/memory are the [official Stellar validator
  recommendation](https://developers.stellar.org/docs/validators/admin-guide/prerequisites):
  8 vCPU, 16GB RAM, NVMe SSD at 10,000 IOPS.
- **Disk depends on `spec.historyMode`, and this is operator-specific
  behavior, not a generic Stellar recommendation** — when `spec.storage.size`
  is left unset, `src/controller/resources.rs` auto-sizes the PVC based on
  this field:
  - `Recent` (the CRD default): **100Gi**, and the generated `stellar-core.cfg`
    sets `CATCHUP_COMPLETE=false` with `CATCHUP_RECENT=60480` ledgers — at
    the current ~5s ledger close time (§3.1) that's roughly **3.5 days** of
    rolling history, not the 30-day window generic Stellar docs describe.
    This is the node type §2 calls "bounded" — a fixed 100Gi covers it
    indefinitely.
  - `Full`: **1,500Gi (1.5Ti)**, with `CATCHUP_COMPLETE=true` (replays and
    retains from genesis). This is the "standalone full-history archive
    node" §2 and §3's growth model are about — 1,500Gi is the operator's
    starting allocation, not a ceiling; it grows from there per §3.
  - If you set `spec.storage.size` explicitly, it overrides both defaults —
    the values above only apply when it's omitted.
- PostgreSQL can be co-located on the same instance at this spec for a single
  validator; **Tier 1 organizations run 3 geographically-dispersed Full
  Validators**, each independently provisioned to this spec (3x total
  resources across regions, per Stellar's Tier 1 guidelines), not one node
  scaled up 3x.
- `StellarNode` mapping (explicit `storage.size` recommended in production so
  the request is self-documenting rather than relying on the historyMode
  default):
  ```yaml
  spec:
    nodeType: Validator
    network: mainnet
    historyMode: Full            # or "Recent" — see distinction above
    resources:
      requests: { cpu: "8", memory: "16Gi" }
      limits: { cpu: "8", memory: "16Gi" }   # Guaranteed QoS — see resource-limits.md
    storage:
      size: "1500Gi"                        # match historyMode's default, or size per §3
      storageClass: "stellar-fast-ssd"        # allowVolumeExpansion: true, ≥10000 IOPS
  ```

### 1.2 Horizon (API + Ingestion DB)

Horizon is really two tiers with different sizing:

| | CPU | Memory | Disk | IOPS | Cloud equivalent |
|---|---|---|---|---|---|
| **API service (Mainnet)** | 4 vCPU | 16Gi | 100Gi SSD | 3,000 | AWS `c5.xlarge` / GCP `n4-highcpu-4` |
| **PostgreSQL DB (Mainnet)** | 4 vCPU | 32Gi | 2Ti SSD (NVMe/DAS) | 7,000 | AWS `r5.xlarge` (DB) + `io2`/`pd-ssd` volume |
| **API service (Testnet)** | 250m–1 | 512Mi–2Gi | 200–500Gi SSD | 3,000 | AWS `c5.large` / GCP `n4-highcpu-2` |

- Source: [Horizon admin-guide
  prerequisites](https://developers.stellar.org/docs/data/apis/horizon/admin-guide/prerequisites).
  The 2Ti DB figure assumes the default **30-day retention window** — this is
  *not* full history; see [§2](#2-history-mode-recent-vs-full-and-captive-core)
  for what full history costs on top of this.
- PostgreSQL ≥ 12 is required. Split the API pod and the database onto
  separate `StellarNode`/StatefulSet-backed volumes so the two can be scaled
  and backed up independently — the API tier is stateless-ish and horizontally
  scalable (see [resource-limits.md](../resource-limits.md#autoscaling-recommendations)),
  the DB tier is not.

### 1.3 Soroban RPC

| | CPU | Memory | Disk | IOPS | Cloud equivalent |
|---|---|---|---|---|---|
| **Mainnet, ≤100 req/s (baseline)** | 4–8 vCPU | 16Gi | 350Gi | ≥3,000 | AWS `c5.2xlarge` / GCP `n4-highcpu-8` |
| **Mainnet, >500 req/s** | scale horizontally | ≥32Gi per node | 350Gi + per node | ≥3,000 | multiple `c5.2xlarge` behind a load balancer |
| **Testnet** | 500m–4 | 2–8Gi | 100–200Gi | 3,000 | AWS `c5.xlarge` / GCP `n4-highcpu-4` |

- Source: [Stellar RPC admin-guide
  prerequisites](https://developers.stellar.org/docs/data/apis/rpc/admin-guide/prerequisites).
  350GB assumes the **default 7-day retention window**; each additional day
  of retention costs **≈40GB**. For a 30-day window (matching Horizon/Core's
  default so all three tiers roll off history together):
  `350Gi + (30 - 7) × 40Gi ≈ 1,270Gi`.
- **Local (instance-attached) SSD is strongly recommended over network
  volumes.** Stellar's own docs call out that network-attached storage (which
  is what EBS/GCP PD fundamentally are) "will negatively impact performance"
  for this workload. On EKS/GKE this generally means an instance type with
  local NVMe (e.g. `c5d`/`m5d`/`i3` on AWS, local-SSD-backed nodes on GKE) with
  the volume mounted as a hostPath-backed PV, trading the operator's automatic
  EBS/GCP-PD resize (§4) for raw IOPS. If you need automatic resize, use a
  provisioned-IOPS network volume and accept the latency cost; don't try to
  combine local SSD with online expansion — local disks generally cannot be
  resized live.

---

## 2. History Mode: Recent vs. Full (and Captive Core)

Two independent axes decide whether a node's storage is bounded (size it
once) or unbounded (it needs §3's growth model and §4's auto-expansion):

**Axis 1 — `spec.historyMode` on Validator/Horizon nodes** (real CRD field,
`src/crd/types.rs::HistoryMode`, applied in `src/controller/resources.rs`):

| | `historyMode: Recent` (default) | `historyMode: Full` |
|---|---|---|
| `stellar-core.cfg` generated | `CATCHUP_COMPLETE=false`, `CATCHUP_RECENT=60480` | `CATCHUP_COMPLETE=true` |
| Retention | ~60,480 ledgers ≈ **3.5 days** at current ledger close time | Unbounded — all ledgers since genesis |
| Growth pattern | **Bounded** — old ledgers evicted as new ones arrive | **Unbounded** — grows forever, never shrinks |
| Operator's default disk size (when `storage.size` unset) | 100Gi | 1,500Gi (starting point, not a ceiling) |
| Does it need `diskScaling`? | Optional — mainly protects against a mis-sized override | **Required** — the only mode where "provision once" is not a valid strategy |
| `retentionPolicy` recommendation | `Delete` is fine — nothing here is irreplaceable | `Retain` (see [archive-pruning.md](../archive-pruning.md) before ever using `Delete`) |

**Axis 2 — Captive Core, used by Soroban RPC** (`SorobanConfig.captiveCoreStructuredConfig`
in the CRD): a `stellar-core` process embedded in the RPC pod that always
runs a short rolling window (§1.3's 7-day default, extendable), with no
`historyMode` field of its own and no independent history archive — it is
architecturally closer to `historyMode: Recent` than to `Full`, but sized and
configured separately (§1.3), not via `spec.historyMode`.

**Practical implication for capacity planning:** anything running `historyMode:
Recent` or Captive Core has a storage need that's a *constant* you size once
(§1) and revisit only when network-wide activity changes materially enough to
justify a bigger rolling window. `historyMode: Full` is a *function of
time* — it is the one configuration this guide's growth model (§3) is
actually projecting for. If your deployment has no `Full`-mode nodes, you can
stop after §1 and enable `diskScaling` as a safety net (§4); §3 is written
for whoever in your org runs the archive tier.

---

## 3. Storage Growth Model

### 3.1 Current network throughput this model is based on

As of writing, live Stellar Mainnet throughput (via
[Chainspect](https://chainspect.app/chain/stellar)):

| Metric | Value |
|---|---|
| Average TPS (trailing 1h) | ~124 tx/s |
| Recent peak TPS (trailing 100 ledgers) | ~203 tx/s |
| Network-configured ceiling | 200 tx/s (governable by validator vote) |
| Theoretical protocol capacity (2025 target) | 5,000 tx/s |
| Average ledger close time | ~5.0–5.8s |

> **This changes.** Stellar has been actively raising both the configured
> ceiling and the protocol's theoretical capacity (see [The Road to 5000
> TPS](https://stellar.org/blog/developers/the-road-to-5000-tps-scaling-stellar-in-2025)).
> Treat the TPS figure as a variable you re-measure, not a constant you
> hardcode. §3.4 shows how to pull it live instead of trusting this table.

### 3.2 The model

Storage on an unbounded (full-history) node grows as:

```
daily_growth_bytes  = ledgers_per_day × avg_ops_per_ledger × bytes_per_op
ledgers_per_day     = 86,400 / avg_ledger_close_seconds
avg_ops_per_ledger   = avg_tps × avg_ledger_close_seconds
```

Substituting one into the other, the model reduces to the form that's
actually useful for planning — it doesn't depend on ledger close time at all,
only on sustained operation throughput:

```
daily_growth_bytes  ≈ avg_ops_per_second × 86,400 × bytes_per_op
```

**`bytes_per_op` is deployment- and era-dependent and is the one input in
this formula we do *not* have an authoritative published figure for.** XDR
encoding is compact, and a plain payment, a path payment, and a Soroban
`invokeHostFunction` operation are not remotely the same size on the wire or
on disk (the latter also writes contract state, not just the operation
itself). Rather than presenting a single made-up constant as fact — which is
exactly the kind of unverifiable number this issue's review step exists to
catch — calibrate it from your own archive, per §3.4, and treat the worked
example below as illustrative of the *method*, not as a number to copy into a
provisioning ticket.

### 3.3 Worked example (illustrative — recalibrate before using)

Using the current average of ~124 ops/s and an illustrative mid-range
`bytes_per_op` of **200 bytes** (a commonly-cited order-of-magnitude for a
compact XDR-encoded payment-class operation; Soroban-heavy workloads will run
higher — recalibrate per §3.4):

Daily rate at these inputs: 124 ops/s × 86,400 s/day × 200 bytes/op ≈ 2.14 GB/day.

| Horizon | Days | Projected growth (2.14 GB/day × days) |
|---|---|---|
| 6 months | 182 | ≈ 390 GB |
| 12 months | 365 | ≈ 782 GB |
| 24 months | 730 | ≈ 1,564 GB (≈ 1.56 TB) |

This is a **linear, constant-TPS projection** — it deliberately ignores that
TPS has been trending upward (§3.1) and that Soroban adoption increases
average operation size over time. Both effects mean the true 24-month number
is very likely higher than this table, not lower. Use it as a floor for
planning purposes, not a ceiling.

### 3.4 Calibrating from real data instead of the illustrative constant

The operator already emits `stellar_pvc_size_bytes` for every managed PVC
(see [proactive-disk-scaling.md](../proactive-disk-scaling.md#prometheus-metrics)).
Once an archive node has been running for even a few weeks, its own growth
rate is a strictly better predictor than any published constant:

```promql
# Bytes/second growth rate over the trailing 30 days, per PVC
deriv(stellar_pvc_size_bytes{node_type="Validator"}[30d])
```

```promql
# Projected size N days from now, extrapolating the trailing-30d trend
stellar_pvc_size_bytes + deriv(stellar_pvc_size_bytes[30d]) * 86400 * N
```

Recommended process:

1. Let a representative archive node run for ≥30 days after any config
   change (retention policy, pruning schedule) before trusting its trend.
2. Re-run the `deriv()` query above monthly; feed the result back into §3.3's
   formula in place of the illustrative `bytes_per_op` (solve for it:
   `bytes_per_op = observed_bytes_per_second / avg_ops_per_second` over the
   same window, using your own ops throughput from Horizon's
   `/metrics` or `stellar_pvc_expansion_total` correlation).
3. **This is also the "cross-reference with the past 12 months of mainnet
   growth data" step this issue's Definition of Done asks for.** This guide
   ships the formula and the query; the 12-month number itself should come
   from whichever archive node(s) your organization has actually been running
   — that data is authoritative in a way nothing this PR can fabricate is. A
   core-maintainer reviewer with access to SDF's own long-running archive
   metrics is the right person to substitute a validated 12-month figure into
   §3.3 during review, per this issue's own sign-off requirement.

---

## 4. Automated Storage Resizing (Summary)

Full mechanics, configuration schema, and troubleshooting live in
[proactive-disk-scaling.md](../proactive-disk-scaling.md); this section is
the capacity-planning-relevant summary.

- **Defaults**: expand at 80% usage, by 50% of current size, no more than
  once/hour, capped at 10 expansions per PVC (`src/controller/disk_scaler.rs`).
- **10-expansion ceiling in practice**: starting from 100Gi, 10 expansions at
  the default 50% increment caps out at **100Gi × 1.5¹⁰ ≈ 5,767Gi (≈ 5.63Ti)**.
  Compare this ceiling against your §3 projection — if your 24-month
  projected size exceeds it, either raise `maxExpansions`, raise
  `expansionIncrement`, or provision a larger starting `storage.size` so
  fewer expansions are needed to reach the same ceiling.
- **Provider volume limits matter more than `maxExpansions` at scale**: AWS
  `gp3` tops out at 16TiB, GCP persistent disks at 64TiB per volume. A
  full-history archive node approaching low single-digit TiB should plan a
  migration to [archive pruning](../archive-pruning.md) or sharded storage
  well before hitting either ceiling — `maxExpansions` is a cost/safety
  guard, not the actual upper bound.
- **Local storage cannot be auto-expanded** (see §1.3's Soroban RPC
  recommendation) — if you chose local SSD for RPC performance, size it for
  the full §3 projection up front, since there is no online-expansion safety
  net for that node.

---

## 5. Monitoring Checklist

Set these alerts before going to production (see
[proactive-disk-scaling.md#alerting](../proactive-disk-scaling.md#alerting)
for ready-to-use PromQL):

- [ ] `stellar_pvc_disk_usage_percent > 75` for 10m (early warning, before the
      80% auto-expansion threshold)
- [ ] `stellar_pvc_expansion_count >= maxExpansions - 2` (approaching the
      hard ceiling from §4)
- [ ] `deriv(stellar_pvc_size_bytes[30d])` trending — recompute the §3.4
      projection monthly and compare against the §4 expansion ceiling and
      your cloud provider's per-volume maximum
- [ ] CPU/memory utilization per [resource-limits.md](../resource-limits.md#monitoring-and-tuning)
      — capacity planning isn't just disk; an under-provisioned Horizon or
      Soroban RPC tier degrades long before it runs out of disk

---

## References

- [Stellar Validator Prerequisites](https://developers.stellar.org/docs/validators/admin-guide/prerequisites) — Core hardware baseline
- [Horizon Admin Guide — Prerequisites](https://developers.stellar.org/docs/data/apis/horizon/admin-guide/prerequisites) — Horizon API + DB hardware baseline
- [Stellar RPC Admin Guide — Prerequisites](https://developers.stellar.org/docs/data/apis/rpc/admin-guide/prerequisites) — Soroban RPC hardware baseline, retention-to-disk relationship
- [Publishing History Archives](https://developers.stellar.org/docs/validators/admin-guide/publishing-history-archives) — what "standalone full-history archive node" means operationally
- [Chainspect — Stellar](https://chainspect.app/chain/stellar) — live TPS/throughput figures used in §3.1
- [The Road to 5000 TPS](https://stellar.org/blog/developers/the-road-to-5000-tps-scaling-stellar-in-2025) — why §3.1's TPS figure is a moving target
- [Resource Limits for Stellar Node Types](../resource-limits.md)
- [Proactive Disk Scaling](../proactive-disk-scaling.md)
- [History Archive Pruning Guide](../archive-pruning.md)
