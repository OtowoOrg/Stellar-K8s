# Lifecycle Hooks Framework

Epic: #1525 · Module: `src/controller/lifecycle_hooks.rs` · Runner: `stellar-hooks`

Workloads declare hooks. The shared `stellar-hooks` runner handles ordering,
timeouts, failure policy and metrics, so each workload only supplies the hook
commands.

```yaml
- name: migrate-db
  phase: setup          # setup | readiness | teardown
  order: 10             # lower runs first; ties broken by name
  command: ["/app/migrate"]
  timeoutSeconds: 60
  idempotent: true      # required for setup and readiness
- name: peers-connected
  phase: readiness
  command: ["/app/check-peers"]
  timeoutSeconds: 3
  idempotent: true
- name: drain
  phase: teardown
  failurePolicy: block  # default for teardown is warn
  command: ["/app/drain"]
  timeoutSeconds: 20
```

## Phases and failure semantics

| Phase       | Wired as                   | Default policy | Blocking failure means     |
|-------------|----------------------------|----------------|----------------------------|
| `setup`     | init container (app image) | `block`        | pod never starts           |
| `readiness` | exec readiness probe       | `block`        | pod stays NotReady         |
| `teardown`  | `preStop` exec handler     | `warn`         | runner exits non-zero      |

Within a phase, hooks run one at a time in `(order, name)` order. After a
`block` hook fails or times out, the remaining hooks in that phase are skipped.
A `warn` failure is recorded and the phase continues.

## Enforcement

`validate` rejects the following:

- duplicate hook names
- empty commands
- zero timeouts
- non-idempotent setup or readiness hooks, because those phases re-run on
  restarts and on every probe
- teardown hooks whose timeouts add up to more than
  `terminationGracePeriodSeconds`

At runtime, each teardown hook's timeout is clipped to whatever is left of the
grace period. Hooks that no longer fit are skipped.

## Wiring and metrics

`apply_to_pod(spec, container, runner_image, hooks)` adds the following to the
pod:

- a `stellar-hooks` emptyDir mounted at `/stellar-hooks`
- an init container that copies the runner into that volume
- the setup init container
- the readiness probe
- the `preStop` handler

The runner is copied into the app container, so it must be able to run there:
build it as a static binary for images that do not use glibc.

Each run writes `/stellar-hooks/metrics/<phase>.prom` for the textfile
collector:

```text
stellar_lifecycle_hook_duration_seconds{hook="migrate-db",phase="setup",outcome="succeeded"} 1.204
stellar_lifecycle_phase_blocked{phase="setup"} 0
```

It also prints the phase result as JSON to stdout.
