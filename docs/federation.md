# Federation

This document describes how to run and operate the federation stack, including
secret synchronization between clusters.

## Secrets Sync

Federation secrets are synchronized with the helper script
`scripts/sync-federation-secrets.sh`. The script is wired into the Makefile so
it can be invoked consistently from local shells and CI.

### `make federation-secrets-sync`

Run the sync against the currently configured cluster:

```sh
make federation-secrets-sync
```

The target delegates to `scripts/sync-federation-secrets.sh` and is guarded on
cluster/secret availability: when the federation stack (or its secrets) is not
present, the target skips cleanly instead of failing. Failure exit codes from
the underlying script propagate, so CI can rely on the target's exit status.

This target is annotated and appears in `make help`.

### CI

The sync is also exposed as a guarded workflow entry point. Automation runs the
same target and skips safely when the federation stack is absent, while still
propagating real failures.

## See Also

- `scripts/sync-federation-secrets.sh` — the underlying sync implementation
- `make help` — lists `federation-secrets-sync` alongside other targets
