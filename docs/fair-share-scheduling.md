# Hierarchical Fair-Share Quotas & Scheduler Extension

> Issue #1959. Code: `src/fair_share/`, binary `stellar-fairshare-extender`, CRD `TenantQuota`
> (`stellar.org/v1alpha1`, cluster-scoped, short name `tq`). The older flat DRF engine in
> `src/fair_share_scheduler.rs` is unchanged.

## What it does

* **Guaranteed minimum** per tenant — protected from preemption.
* **Elastic maximum** — idle capacity may be borrowed up to `max`, and is reclaimed by preemption when
  the owner needs its guarantee back.
* **Hierarchy** — tenants are grouped (`parent`); a group's `max` bounds its whole subtree, a group's
  guarantee is at least the sum of its children's.
* **ResourceQuota stays** as a coarse safety net (see [Migration](#migration-from-namespace-quotas)).

## Declaring the tree

```yaml
apiVersion: stellar.org/v1alpha1
kind: TenantQuota
metadata: {name: platform-core}
spec:
  parent: platform            # optional; omit for a root
  namespaces: [core-validators] # leaves only; each namespace in exactly one node
  weight: 2                   # share of *elastic* capacity among siblings (> 0)
  guaranteed: {cpu: "30", memory: 120Gi}
  max: {cpu: "100", memory: 400Gi}   # optional: inherits the parent's
```

Full example: `config/fair-share/sample-quota-tree.yaml`.

### Validation (all enforced, with the offending node named)

duplicate names, missing parent / self-parent / **cycles**, a namespace claimed twice, namespaces on a
non-leaf, invalid namespace names, `weight <= 0` or non-finite, unparsable quantities,
`guaranteed > max`, a child `max` above its parent's, a declared parent guarantee **below the sum of its
children's**. The sum of root guarantees is additionally checked against cluster capacity
(warning in the extender log; allocation scales guarantees down and flags `oversubscribed` rather than
silently approximating).

An invalid update **never replaces** the active configuration: the extender keeps the last known good tree
and sets `stellar_fairshare_config_valid 0`. Lowering a `max` never evicts running pods; it only blocks new ones.

## Algorithm

Guarantees and maxima are **vectors** (CPU, memory). A pod is "inside the guarantee" only if it fits in
*every* dimension; the part above is **elastic** in that dimension.

**Entitlement** (`Tree::entitlements`) — weighted max-min fairness with guarantees and caps (progressive
filling), per dimension, top-down from cluster capacity. Each child first gets `min(guarantee, demand)`;
the rest is shared by `weight` among children that still want more, up to `min(max, demand)`, and
capacity freed by saturated children is redistributed. Integer arithmetic only, so a guarantee is never
approximated below its configured value (property-tested). This is *not* Dominant Resource Fairness across
dimensions; dimensions are allocated independently.

**Admission** (`/filter`) for pod `P` of tenant leaf `L`:

1. **Ceilings.** For `L` and every ancestor: `usage + req <= max`. If a pod that is entirely inside `L`'s
   guarantee trips only an *ancestor's* ceiling (siblings borrowed), the verdict is **NeedsReclaim**
   (resolvable by preemption, reported as `failedNodes`); an elastic request that trips a ceiling is a
   hard **DenyMax** (`failedAndUnresolvableNodes`).
2. **Guarantee.** The part of `req` within `L`'s remaining guarantee is always admitted.
3. **Elastic part** is admitted only if it fits in free cluster capacity **after reserving** the unmet
   guarantees of other tenants that have pending pods, and — while any other tenant is waiting — only up to
   `L`'s weighted entitlement. (A scheduler extender cannot reorder the queue, so fairness is enforced at
   admission.)

**Preemption** (`/preempt`, and `Engine::select_victims`):

* only a pod that fits entirely inside its tenant's remaining guarantee may preempt;
* a victim's tenant, and every node from the victim's leaf up to (excluding) the **lowest common
  ancestor** with the preemptor, must stay **at or above its guarantee** after the eviction — sibling and
  cousin groups at/below their guarantee are protected;
* victims from the most over-guarantee tenants go first; evictions must also **cure any exceeded ancestor
  ceiling** (otherwise the extender vetoes the node);
* within one tenant only strictly lower-priority pods may be evicted (equal-priority self-eviction is a
  zero-sum swap that previously caused endless churn — found by the simulation, fixed and tested).

## Components

| Piece | Role |
|---|---|
| `TenantQuota` CRD | declarative tree (`stellar-fairshare-extender --print-crd`) |
| `stellar-fairshare-extender` | polls quotas/pods/nodes (2 s), serves `/filter`, `/preempt`, `/metrics`, `/healthz` |
| `config/fair-share/scheduler-config.yaml` | `KubeSchedulerConfiguration` `extenders:` stanza |
| `monitoring/fairshare-alerts.yaml` | config rejected, fairness, saturation, refusals, delay |

### Why an extender, not an in-tree plugin

This repository is Rust; kube-scheduler framework plugins are Go and are compiled into a scheduler build.
The supported way to extend an unmodified kube-scheduler from another language is the **extender** HTTP
protocol (filter / preempt). Consequences, stated plainly:

* upstream kube-scheduler is **not forked or modified**;
* the **upstream scheduler-plugin conformance suite does not apply** to an extender, so that acceptance
  criterion is **not met** (see the PR);
* an extender cannot reorder the scheduling queue (QueueSort) or score pods; fairness is enforced at
  admission as described above;
* the extender's view is refreshed every 2 s, so a burst of pods inside one interval can briefly overshoot
  a ceiling (ResourceQuota remains the hard backstop); `ignorable: true` makes it fail-open.

Preemption decisions are only taken for victims the default scheduler proposes (lower-priority pods); use
uniform PriorityClasses across tenants or borrowed capacity held by higher-priority pods will not be reclaimable.

## Metrics

Tenant labels are bounded by the number of `TenantQuota` objects (admin-defined); pods/namespaces are never labels.

`stellar_fairshare_{allocated,guaranteed,max,entitlement,elastic_usage,guaranteed_utilization_ratio,quota_saturation_ratio}{tenant,resource}`,
`stellar_fairshare_jain_index`, `stellar_fairshare_cluster_used_ratio`, `stellar_fairshare_admission_total{decision}`,
`stellar_fairshare_preemption_total{decision}`, `stellar_fairshare_scheduling_delay_seconds`,
`stellar_fairshare_config_{valid,reloads_total,rejected_total}`.

## Migration from namespace quotas

1. **Observe.** Create `TenantQuota` objects mirroring today's limits (`guaranteed` = typical use, `max` = current
   ResourceQuota hard). Deploy the extender **without** the scheduler config; watch `/metrics`.
2. **Check coexistence.** `check_resource_quota_coexistence` reports namespaces where the ResourceQuota would
   fight the tree: hard < guarantee (error — the safety net would block promised capacity) or hard < max
   (warning — elasticity silently capped). Raise ResourceQuota hard limits to ≥ `max` (it is only a backstop now).
3. **Enable** the extender in kube-scheduler (`ignorable: true` first). Verify `stellar_fairshare_admission_total`.
4. **Tighten.** Move to `ignorable: false` once stable.

**Rollback:** remove the `extenders:` stanza (or set the extender unreachable with `ignorable: true`); ResourceQuota
objects were never removed, so enforcement reverts to the previous behaviour immediately.

## Evidence and what is NOT proven

Run `cargo test --test fair_share` (28 tests) and `cargo test --release --test fair_share -- --ignored fairshare_soak --nocapture`
(`FAIRSHARE_SOAK_TICKS=86400` simulates 24 h of logical time).

The 500-pod / 20-namespace / 7-tenant / 3-group saturation test and the soak run on a **discrete-time simulation**
of the scheduler driving the real policy engine (FIFO queue, first-fit nodes, the engine's admission and
preemption). They verify: no tenant exceeds any ceiling at any tick, preemption never breaches a guarantee
below the LCA (independently re-checked), pending pods that fit in their guarantee get scheduled, and the Jain
index of time-averaged usage/entitlement over saturated ticks.

### Two different "guarantee" invariants

* **Never evicted below a guarantee** (hard): asserted on every tick of the soak — in a simulated 24 h run
  (86 400 ticks, 51 968 completed pods, 4 946 preemptions, 95 % of ticks saturated) there were **zero**
  ceiling breaches and **zero** preemptions that took a node on the victim's path below its guarantee;
  Jain index 0.9997. (The simulation iterates a `HashMap`, so counts vary slightly between runs; an earlier run gave Jain 0.9995.)
* **A pod inside its guarantee is scheduled immediately** (not guaranteed): guarantees are *vectors*. If the
  surplus that could be reclaimed is memory held by tenants that are themselves below their CPU guarantee,
  evicting their pods would breach *their* guarantee, so it is refused; and preemption is node-local. The
  waiting tenant then runs once capacity turns over. The soak's memory-skewed workload (up to 8 GiB per CPU
  against 4 GiB per CPU guarantees) produced a worst case of **101 simulated seconds** (138 in an earlier run); the test bounds this
  by pod lifetime (≤ 240 s). Keep guarantee CPU:memory ratios close to your workloads' request ratios.

**Not done:** a wall-clock 24 h test on a real cluster, and the upstream scheduler-plugin conformance suite.
Real-cluster effects (API latency, the 2 s refresh window, graceful termination, PDBs on victims) are not simulated.
