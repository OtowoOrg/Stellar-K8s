# Progressive Config Rollout

Epic: #1998 · Engine: `src/progressive_config.rs` · Read side: `src/config_rollout_registry.rs` · API: `src/rest_api/config_rollout_handlers.rs`

Operator settings are rolled out as versioned bundles to a small canary first.
A failing health gate stops the rollout and restores the previous bundle, and
the config version every component runs can be queried at any time.

## Guarantees

| Property | Value |
|----------|-------|
| Canary blast radius | at most **5%** of the target set (`MAX_BLAST_RADIUS`), chosen deterministically |
| Rollback | the previous bundle is restored automatically on a gate failure, measured against a **30 s** SLA (`ROLLBACK_SLA_MS`) |
| Advancing | a stage is never entered without a gate pass |
| Containment | a gate failure never propagates beyond the canary subset |
| Visibility | every component records the bundle version it runs |

## Lifecycle

```text
Idle -> Canary -> Propagating -> Complete
           \
            -> RolledBack   (a gate failed; the previous bundle is restored)
```

1. `ProgressiveConfigRollout::start(bundle, apply)` applies the bundle to the
   canary subset only.
2. `gate_or_rollback(samples, source, apply)` evaluates the health signals
   against the gates (`default_gates()` or a component-specific set from
   `with_gates`). One failed gate fails the window. (`evaluate` alone only judges
   the window.)
3. On failure the previous bundle is restored on the canary, the restore time is
   recorded in a `RollbackRecord` (`within_sla` says whether it met 30 s), and the
   stage becomes `RolledBack`. `rollback` can also be called directly and is
   idempotent.
4. On a pass the stage becomes `Propagating`, and `complete(apply)` applies the
   bundle to the rest of the fleet.

A `ConfigBundle` is a version plus its settings, with a content digest, so
"which settings is this node on?" has an exact answer.

## Querying versions

Whoever drives a rollout publishes a snapshot after each step:

```rust
use stellar_k8s::config_rollout_registry;

config_rollout_registry::global().publish_rollout("operator-settings", "2.4.0", &rollout);
```

The REST API (behind the normal `api_reader` auth) reads those snapshots:

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/config/rollouts` | Every rollout: stage, blast radius, canary set, failing gates, last rollback |
| GET | `/api/v1/config/rollouts/:id` | One rollout |
| GET | `/api/v1/config/versions` | The config version each component runs; optional `?status=canary\|restored\|promoted` |
| GET | `/api/v1/config/versions/:namespace/:node` | One component's config version |

```bash
curl https://operator:9090/api/v1/config/versions/stellar/validator-1
```

```json
{
  "component": "stellar-core",
  "namespace": "stellar",
  "node": "validator-1",
  "version": "2.4.0",
  "digest": "9f2c…",
  "status": "canary",
  "updated_at": "2026-11-19T10:15:00Z"
}
```

`status` is `canary` while the node runs the candidate bundle, `restored` after
a rollback and `promoted` once the bundle is fleet-wide. When several rollouts
have touched the same component, the most recently published one is reported.

## Where this stops

The engine, the registry and the API are in place. This change does not add a
controller loop that triggers rollouts from a CR or ConfigMap: something has to
call `start`, `evaluate` and `publish_rollout`. Until that exists, the registry
is empty and the version endpoints return `404` for components no rollout has
touched.
