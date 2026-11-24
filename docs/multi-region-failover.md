# Multi-Region Failover Orchestration with Health-Gated Traffic Shift

Epic #1504. Moves a public hostname between regions **incrementally**, under a
**health gate**, and brings it back automatically once the primary recovers —
with every step recorded in a controller-owned plan CR.

- CRD: `TrafficShiftPlan` (`src/crd/traffic_shift_plan.rs`)
- Controller: `traffic_shift` (`src/controller/traffic_shift.rs`)
- Wiring: `src/controller/mod.rs`, `src/crd/mod.rs`, `src/controller/metrics.rs`

## Why a plan CR instead of a flag

DNS is the slowest, least reversible part of a failover. The operator therefore
never edits a record imperatively. It renders a **weighted routing record** from
the plan's spec plus its own progress, publishes that document, and records the
exact JSON it published in `status.steps[].record` and `status.appliedRecord`.
The same JSON is mirrored into the `stellar.org/applied-traffic-record`
annotation, so the live DNS state is visible with `kubectl get trafficshiftplan`.

The rendered record is an external-dns `DNSEndpoint` with per-target weights
that always sum to 100, so a resolver never observes a partially-applied shift:

```yaml
spec:
  endpoints:
    - dnsName: horizon.stellar.example.com
      recordType: A
      recordTTL: 60
      targets:
        - target: horizon.eu-west.example.com
          weight: 75
        - target: horizon.us-east.example.com
          weight: 25
```

## The health gate

`evaluate_gate` scores the two regions **independently** and is the *only*
thing that can open a shift. For `Failover` the primary must prove it is down
(`failureThreshold` consecutive failures) and the secondary must prove it is up
(`recoveryThreshold` consecutive successes, at least `minSamples` inside the
evidence window, at least `minSuccessRatePercent`, and evidence no older than
`evidenceWindowSeconds`).

**Failback uses the identical function with the roles swapped.** There is no
second, laxer bar for returning traffic to the primary — the test
`failback_needs_exactly_the_same_evidence_as_failover` asserts the `applied`
thresholds are identical in both directions, and
`failback_does_not_flip_back_on_a_flapping_primary` asserts a merely noisy
primary never pulls traffic back.

Every decision is recorded on the plan (`status.lastGate`, and the gate that
authorised each step) with the raw evidence for both regions, so "why did we
fail over at 03:14" is answerable from the object.

## The state machine

```
Idle ──gate open──▶ Draining ──drain elapsed──▶ Shifting ──▶ Soaking ─┐
  ▲                   ▲                                            │
  │                   └──────── gate closed / budget blown ─────────┤
  │                                                                 │
  └────────────────────────── Completed ◀───────────────────────────┘
```

1. **Draining** — no weight moves while the region losing traffic drains its
   connections (`spec.shift.drainSeconds`).
2. **Shifting** — one increment of `spec.shift.stepPercent` is published.
3. **Soaking** — the shift waits `max(recordTTL, spec.shift.soakSeconds)` before
   the next increment. The TTL is the floor, not the soak: a soak shorter than
   the TTL would advance the shift before resolvers could have seen the previous
   increment. If the region's mean error rate over the soak exceeds
   `maxSoakErrorRatePercent`, the shift is **aborted** and the weights are held
   at the last safe increment — never rolled back, never pushed further.
4. **Completed** — the target weights are published and RTO is measured.

Weight is never moved while the gate is closed, and an in-flight increment
always finishes its drain and soak before the state machine looks at anything
else, including the target.

## DNS TTL and connection draining

Both are modelled explicitly rather than left to a sleep:

- `drain_deadline(spec, started_at)` — the losing region's drain window.
- `propagation_deadline(spec, published_at)` — `published_at + max(ttl, soak)`.
- `projected_duration_secs(spec, from, to)` — `steps × (drain + max(ttl, soak))`.

`validate_plan` rejects a plan whose projected duration exceeds its own declared
RTO: such a plan could never be compliant, so it is failed up front instead of
failing a real incident.

## RTO, RPO and the DR compliance report

- `measure_rto` compares the shift's wall clock against `spec.targets.rtoSeconds`
  and reports `overBySeconds` when it is missed — a miss is surfaced, not hidden.
- `rpo_evidence` reads the secondary's replication lag and compares it against
  `spec.targets.rpoSeconds`; a breach raises a `Degraded / RpoTargetMissed`
  condition.
- `drill_compliance_record(plan)` renders one JSON entry per plan — measured
  RTO/RPO, every step and its outcome, and whether the declared targets were
  met. Quarterly full-region drills (`spec.targets.drill: true`, tagged with
  `spec.drillId`) feed this straight into the DR compliance report.

## Operator levers

| Lever | Effect |
| --- | --- |
| `spec.trigger: Manual` | the plan will not start on gate evidence alone |
| `stellar.org/trigger-failover: "true"` | explicit start for a `Manual` plan |
| `stellar.org/suppress-shift: failover,failback` | pause a direction without editing the spec |

## Metrics

| Metric | Meaning |
| --- | --- |
| `stellar_traffic_shift_phase` | plan phase, by direction (0=Idle … 7=Failed) |
| `stellar_traffic_shift_primary_weight_percent` | share of traffic still on the primary |
| `stellar_traffic_shift_rto_seconds` | measured RTO of the last completed shift |

## Tests

`src/controller/traffic_shift_test.rs` drives the gate and the state machine
against a fixture clock — no cluster, no network:

- gate independence, minimum samples, success rate, stale evidence;
- failback gated by identical evidence; flapping primary rejected;
- full failover walk: four increments, drains and soaks honoured, steps recorded;
- gate closing mid-shift holds the last safe increment;
- soak error budget aborts and later resumes;
- drain completes before any weight moves; increments never exceed the step size;
- step arithmetic, TTL vs soak, RTO feasibility validation;
- RTO/RPO measurement and the drill compliance record.

Run them with `cargo test --lib traffic_shift`.
