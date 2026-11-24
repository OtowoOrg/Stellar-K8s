# Feature Flag Rollouts

Issue #1337. Source: [`src/feature_flags.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/feature_flags.rs).

## Two flag systems, different jobs

| | `controller::feature_flags` | `feature_flags` (this) |
|---|---|---|
| Answers | Is this cluster-wide capability on? | Is this on **for this subject**? |
| Shape | `bool` | Percentage + segments + allow/deny |
| Example | `enable_dr` | `new_archive_pruner` at 25% in staging |

Both read the same `stellar-operator-config` ConfigMap, so a flag is toggled
by editing that ConfigMap — no rebuild, no redeploy, no restart.

## Evaluation order

A rule is evaluated in a fixed precedence, and `Decision::reason` always
reports which step decided:

1. **Flag missing** → off. An unknown flag is never on by accident.
2. **`enabled: false`** → off. The master switch is the kill switch.
3. **Deny list** → off. Wins over everything below, including the allow list.
4. **Allow list** → on, skipping segments and percentage.
5. **Segments** → all must match, or off.
6. **Percentage** → deterministic bucketing.

Deny beats allow deliberately: an explicit exclusion is a safety valve and must
not be defeated by the same subject also being allow-listed.

## Deterministic bucketing

A subject's bucket is `fnv1a64("{flag}:{subject}") % 10_000`, and the flag is on
when `bucket < percentage * 100`. Three properties matter:

- **Stable** — the same subject always lands in the same bucket, so a user does
  not see the feature flicker between requests or replicas.
- **Monotonic** — raising the percentage only ever *adds* subjects. Nobody
  loses the feature because the rollout widened, which is what makes a staged
  1% → 10% → 50% ramp safe.
- **Independent per flag** — the flag name is hashed with the subject, so a
  subject is not permanently in the first 1% of every flag.

FNV-1a is used rather than `DefaultHasher`, whose output is explicitly not
stable across releases — bucketing on it would reshuffle every rollout on
upgrade.

The `subject` must be **stable** for the entity being rolled out to: a tenant
id, node name, or account id. A per-request value such as a request id would
re-bucket on every call and make the rollout look random.

## Configuring a rollout

### Via Helm values

```yaml
featureFlags:
  rollouts:
    new_archive_pruner:
      enabled: true
      rollout_percentage: 25
      segments:
        - key: env
          op: in            # in | notIn | contains | prefix
          values: [staging, canary]
      allow_subjects: [tenant-alpha]
      deny_subjects: [tenant-critical]
```

Each entry renders a `flag.<name>` key into the ConfigMap.

### Live, without a deployment

```bash
kubectl -n stellar-system edit configmap stellar-operator-config
```

Change `rollout_percentage` and save. The operator's ConfigMap watcher picks it
up; no pod restart.

To halt a rollout immediately, set `"enabled": false` — the master switch
overrides percentage and allow lists.

## Usage

```rust
use stellar_k8s::feature_flags::{EvaluationContext, FlagSet};

let (flags, warnings) = FlagSet::from_config_map_data(&configmap_data);
for warning in &warnings {
    tracing::warn!("{warning}");
}

let ctx = EvaluationContext::new(tenant_id)
    .with_attribute("env", "staging")
    .with_attribute("region", "eu");

if flags.is_enabled("new_archive_pruner", &ctx) {
    new_pruner().await?;
} else {
    legacy_pruner().await?;
}

// Or, to explain the outcome:
let decision = flags.evaluate("new_archive_pruner", &ctx);
tracing::info!(
    enabled = decision.enabled,
    bucket = ?decision.bucket,
    "flag decided: {}", decision.reason
);
```

A malformed flag definition is skipped with a warning rather than failing the
whole reload — one bad edit must not drop every other flag.

## Verification

```bash
K8S_OPENAPI_ENABLED_VERSION=1.30 cargo test --lib feature_flags::
K8S_OPENAPI_ENABLED_VERSION=1.30 cargo test --doc feature_flags
```

55 tests. The ones that speak to this issue's acceptance criteria:

| Test | Proves |
|---|---|
| `a_partial_rollout_hits_approximately_the_configured_share` | 25% reaches 23–27% of 10,000 subjects |
| `raising_the_percentage_only_ever_adds_subjects` | Monotonic across a 1→5→10→25→50→100% ramp |
| `a_subject_gets_the_same_answer_on_every_replica` | Independent evaluators agree |
| `the_same_subject_buckets_differently_per_flag` | No cross-flag correlation |
| `buckets_are_reasonably_uniform` | Every decile within ±25% of even |
| `fnv1a_matches_the_published_vectors` | Hash pinned, so a refactor cannot reshuffle rollouts |
| `editing_the_configmap_changes_the_answer` | Toggling without redeploying |
| `the_deny_list_wins_over_the_allow_list` | Precedence |
| `segments_and_percentage_compose` | Segment targeting gates the percentage |

Check the rendered ConfigMap:

```bash
helm template stellar-operator charts/stellar-operator \
  --set 'featureFlags.rollouts.demo.enabled=true' \
  --set 'featureFlags.rollouts.demo.rollout_percentage=25' \
  | grep -A6 'flag.demo'
```
