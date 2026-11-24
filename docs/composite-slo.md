# Composite Service SLOs

Epic: #1524 · Config: `config/slo/composite-slos.yaml` · Rules: `monitoring/composite-slo-rules.yaml` · Module: `src/composite_slo.rs`

A composite SLO combines a service's existing SLI ratio series into one
weighted objective:

```text
stellar:slo:composite:ratio{service="stellar-api"}
  = 0.5 · availability + 0.3 · latency + 0.2 · error-rate
```

The composite is published through recording rules, so existing alerting and
burn-rate tooling can consume it unchanged:

| Series | Meaning |
|--------|---------|
| `stellar:slo:composite:ratio` | weighted composite, evaluated every 1m |
| `stellar:slo:composite:ratio_rate{1h,6h,1d,3d,30d}` | composite averaged over the window |
| `stellar:slo:composite:burn_rate{1h,6h,1d,3d,30d}` | `(1 − ratio_rate) / (1 − target)` |
| `stellar:slo:composite:error_budget_remaining` | `1 − burn_rate30d` |
| `stellar:slo:composite:weights_version` | reviewed weighting version in effect |

Dashboards and alerts read these pre-recorded series, so queries stay cheap.

## Changing weights

1. Edit the SLI weights in `config/slo/composite-slos.yaml`. The weights must
   sum to 1.
2. Bump `version`.
3. Append a `reviews` entry with the same version, `reviewedBy`, `reason` and
   the new weights.
4. Regenerate the rules with
   `STELLAR_REGENERATE_SLO_RULES=1 cargo test --lib composite_slo`.

`cargo test` fails if any of the following is true:

- a tier-1 service has no composite
- an SLI references a recording rule that does not exist
- the weights do not sum to 1
- the current weights do not match the latest review record
- the committed rules file is out of date

CODEOWNERS requires `@observability-team` to review `config/slo/`.

## Verification

A unit test builds 30 days of per-minute samples for each SLI, including an
incident. It checks that the error-budget burn computed from the composite
stays within 1% of the manual weighted multi-SLI calculation.
