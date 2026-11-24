# Configuration Reference

Configuration options and references for Stellar-K8s.

This page documents the configuration the **operator generates** for a
`StellarNode` and the **advanced keys you may add yourself**. For the
authoritative list of CRD fields see [CRD Reference](crd-reference.md).

## How configuration reaches the node

The operator reconciles a `StellarNode` into two independent configuration
surfaces, with different owners and different lifetimes.

| Surface | Where it lives | Owner | How to change it |
| --- | --- | --- | --- |
| `stellar-core.cfg` and friends | A `ConfigMap` named `<node>-config`, mounted read-only at `/config` | Operator | Edit the `StellarNode` spec; never edit the `ConfigMap` |
| Container environment | Rendered directly into the container spec | Operator | `spec.stellarCoreEnv` / `spec.horizonEnv`, or the seed fields |

!!! warning "Do not hand-edit the generated ConfigMap"

    The operator reconciles `spec` into the `ConfigMap` on every pass. A manual
    `kubectl edit configmap` is reverted on the next reconcile loop, and the
    resulting config drift is difficult to diagnose. Change the `StellarNode`
    instead.

## Generated ConfigMap keys

These are the only keys the operator writes. Anything else you see in the
`ConfigMap` was put there by hand and will not survive a reconcile.

| Key | Node type | Default | Source |
| --- | --- | --- | --- |
| `NETWORK_PASSPHRASE` | all | derived from `spec.network` | `spec.network`, or `spec.customNetworkPassphrase` / `spec.passphraseSecretRef` |
| `MTLS_ENABLED` | all | *absent* | Present only when mTLS is enabled |
| `stellar-core.cfg` | Validator | *absent* | Present when mTLS, a history mode, or user TOML is set |
| `STELLAR_CORE_URL` | Horizon, SorobanRpc | *absent* | `spec.horizonConfig.stellarCoreUrl` |
| `INGEST` | Horizon | *absent* | `spec.horizonConfig.enableIngest`, forced to `false` under leader election or `replicas > 1` |
| `captive-core.cfg` | Horizon, SorobanRpc | *absent* | Rendered from the captive-core config when set |
| `ebpf-exporter.yaml` | all | *absent* | Present when `spec.ebpfConfig.enabled` |

`NETWORK_PASSPHRASE` is also injected as a container environment variable
alongside the `ConfigMap` key. The spec has no `networkPassphrase` field — the
passphrase is resolved from `spec.network` (`Mainnet`, `Testnet`, `Futurenet`,
or `Custom`), with `spec.customNetworkPassphrase` and
`spec.passphraseSecretRef` supplying the value for a custom network.

`INGEST` deserves a note: when `enableIngestionLeaderElection` is set, or the
node has more than one replica, the operator forces `INGEST=false` so only the
elected leader ingests. Do not set it yourself expecting it to override this.

## `stellar-core.cfg` structure

`stellar-core.cfg` mixes *root* keys (`CATCHUP_COMPLETE`, `TLS_CERT_FILE`, …)
with *table* sections such as `[QUORUM_SET]`, `[[VALIDATORS]]` and
`[[HOME_DOMAINS]]`. In TOML, every key written **after** a table header belongs
to that table — so a bare key that follows a user's `[[VALIDATORS]]` header is
silently swallowed by that table, and `stellar-core` ignores it with no error.

The operator defends against this by always emitting its own keys first, before
any user content, separated by a marker comment:

```toml
# mTLS Configuration (best-effort; see docs/mtls-guide.md)
HTTP_PORT_SECURE=true
TLS_CERT_FILE="/etc/stellar/tls/tls.crt"
TLS_KEY_FILE="/etc/stellar/tls/tls.key"
# Recent History Mode
CATCHUP_COMPLETE=false
CATCHUP_RECENT=60480

# ---- user-supplied configuration (below) ----
# <your spec.validatorConfig.quorumSet content>
```

The operator-managed root keys it may write are `CATCHUP_COMPLETE`,
`CATCHUP_RECENT`, `HTTP_PORT_SECURE`, `KNOWN_PEERS`, `TLS_CERT_FILE`, and
`TLS_KEY_FILE`. Your own content is emitted verbatim after the marker.

### Root keys versus table keys

A list of `stellar-core.cfg` root keys is tracked by the operator, which lets it
tell a genuine mistake apart from a legitimate table key. It is recognised as a
root key, and is **not** written by the operator:

`ARTIFICIALLY_CATCHING_UP`, `AUTO_AUTH`, `BUCKET_LIST_SIZE_LIMIT`,
`CATCHUP_SKIP`, `CONFIGURE_DNS`, `DATABASE`, `DISABLE_AUTO_UPDATE`,
`DISABLE_HISTORICAL_LEDGER_CHECKPOINT_ELISION`, `DISABLE_SCP`,
`DISABLE_XDR_DEBUG`, `ENABLE_DEBUG_HTTP_ENDPOINTS`, `ENABLE_OTEL`, `HTTP_PORT`,
`LEDGER_CLOSE_LATENCY_MS`, `LEDGER_STATE_UPPER_BOUND`, `LEDGER_VALIDITY_LEDGERS`,
**`LOG_FILE_PATH`**, `LOG_LEVEL`, `LOG_LINE_LIMIT`, `MAX_BACK_HISTORY_OBJECTS`,
`MEMORY_LIMIT_MODE`, `METADATA_SERVER_PORT`, `METADATA_SERVER_URL`,
`METADATA_STREAM_CACHE_SIZE`, `METADATA_STREAM_CACHE_UPDATE_PERIOD_MS`,
`MINIMUM_STATE_LEDGER`, `MIN_TEMP_PEERS`, `MODE`, `NODE_NAMES`, `NODE_SEED`,
`OBSERVING_PORT`, `PEER_PORT`, `PEER_PORT_SECURE`, `PORT`, `PREVENT_CRAPFALL`,
`PUBLIC_HTTPS_PORT`, `PUBLIC_HTTP_PORT`, `RESPONSE_OVERHEAD_MS`, `SECRET_SEED`,
`SNAPSHOT_FILE`, `SOURCE_DIR`, `STORAGE_TYPE`, `USE_CONFIG_TOML`,
`USE_HISTORICAL_CACHE`, `USE_TOML_CFGS`, `WORKER_THREADS`.

### When a key is in the wrong place

After assembling the document the operator re-parses it and checks each key
against that list. If you write a root key **below** a table header it logs a
warning naming the key, its line, and the table that captured it:

```text
StellarNode <name>: key LOG_FILE_PATH on line 42 is scoped into table
[VALIDATORS]; move it above the first [[TABLE]] header if it was meant to be a
stellar-core.cfg root key
```

If an *operator* key ever ends up table-scoped it logs:

```text
StellarNode <name>: operator key TLS_CERT_FILE was scoped into table
[VALIDATORS][0] instead of the stellar-core.cfg root; it will be ignored by
stellar-core
```

These are log warnings only. They raise no status condition, and the node still
reconciles — the mis-scoped key is simply not applied. **Watch the operator
logs after changing `spec.validatorConfig.quorumSet`**, because the failure mode
is otherwise a validator that starts with the wrong settings and no complaint.

## `LOG_FILE_PATH`

**The operator does not set `LOG_FILE_PATH`.** It is a key you supply. This is
worth stating plainly because a validator that appears to be logging to disk is,
by default, not writing a log file at all — see [Log output and
rotation](#log-output-and-rotation).

| Aspect | Value |
| --- | --- |
| Key | `LOG_FILE_PATH` |
| Set by operator | No — supply it yourself |
| Operator default | None; the `stellar-core` image default applies |
| Recommended value | `/opt/stellar/data/log/stellar-core.log` |
| Writable only under | `/opt/stellar/data` (the PVC mount), or a volume you add |

Because the root filesystem is read-only, a `LOG_FILE_PATH` pointing anywhere
outside a writable volume — `/var/log/stellar-core.log`, for example — makes
`stellar-core` fail at startup. Place the file under the data volume.

## `BUCKET_DIR_PATH`

Unlike `LOG_FILE_PATH`, the operator **does** set `BUCKET_DIR_PATH` — but in
`captive-core.cfg` (Soroban RPC and Horizon ingestion), not in
`stellar-core.cfg`, and with a default that is **not** on the persistent volume.

| Aspect | Value |
| --- | --- |
| Key | `BUCKET_DIR_PATH` |
| Set by operator | Yes, in `captive-core.cfg` |
| Operator default | `/var/lib/stellar/buckets` |
| Override | The captive-core config's `bucketDirPath` field |
| Writable only under | `/data` (the PVC mount) for Horizon and SorobanRpc |

!!! warning "The captive-core defaults are outside the writable volume"

    `captive-core.cfg` defaults `DATABASE` to
    `sqlite3:///var/lib/stellar/captive-core/stellar.db`, `BUCKET_DIR_PATH` to
    `/var/lib/stellar/buckets`, and `TMP_DIR_PATH` to `/var/lib/stellar/tmp`.
    But the data volume for Horizon and SorobanRpc is mounted at `/data`, and
    the container runs with a read-only root filesystem — so those three
    default paths are not writable.

    If you rely on the defaults, captive-core cannot write its ledger, buckets,
    or temporary files. Point all three under `/data`:

    ```yaml
    spec:
      sorobanConfig:
        captiveCoreStructuredConfig:
          database: "sqlite3:///data/captive-core/stellar.db"
          bucketDirPath: /data/buckets
          tmpDirPath: /data/tmp
    ```

    The exact field names depend on whether you use the structured config or the
    deprecated free-form `captiveCoreConfig` string; the operator renders either
    one into `captive-core.cfg`. See [CRD Reference](crd-reference.md).

## Supplying these keys

For `stellar-core.cfg` root keys, put them in
`spec.validatorConfig.quorumSet` **above** any table header. For container
environment variables, use `spec.stellarCoreEnv`.

```yaml
apiVersion: stellar.otowo.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-mainnet
spec:
  nodeType: Validator
  historyMode: Full
  validatorConfig:
    # Root keys MUST come before any [[TABLE]] header, or the operator logs a
    # scope warning and stellar-core ignores them.
    quorumSet: |
      LOG_FILE_PATH=/opt/stellar/data/log/stellar-core.log
      LEDGER_CACHE_SIZE=16384

      [QUORUM_SET]
      THRESHOLD_PERCENT=67
      VALIDATORS="..."  # your quorum
  stellarCoreEnv:
    # Container environment variables are keyed by name and replace any
    # operator-generated entry of the same name.
    - name: MY_TUNABLE
      value: "on"
```

Notes on this pattern:

- `spec.stellarCoreEnv` entries **replace** any operator-generated entry of the
  same name in place, rather than appending a second one. See
  [Operator environment](operators.md#seed-environment-variables) for the full
  override rules and the one case where a duplicate *does* appear.
- Values must be strings. Quote numeric values so YAML does not coerce them to a
  number the API rejects.
- `stellarCoreEnv` applies to Validator pods only. `spec.horizonEnv` is the
  Horizon equivalent, and SorobanRpc has no environment override field at all.

### Verifying the rendered values

`stellarCoreEnv` entries land in the pod spec:

```bash
kubectl -n <namespace> get pod <node>-0 \
  -o jsonpath='{.spec.containers[?(@.name=="stellar-node")].env}'
```

To inspect the generated config itself:

```bash
kubectl -n <namespace> get configmap <node>-config \
  -o jsonpath='{.data.stellar-core\.cfg}'
```

## Log output and rotation

`LOG_FILE_PATH` writes an **append-only file on the ledger volume**. Two
consequences follow, and both are capacity-planning concerns rather than
correctness ones:

- **The log competes with the ledger for the same PVC.** Ledger growth and log
  growth draw on one allocation, and the PVC is what you pay for. Size
  `spec.storage.size` with headroom for both.
- **Nothing rotates the file.** The operator does not truncate, compress, or
  ship `LOG_FILE_PATH` output, and it is not removed when the pod is deleted,
  because it lives on the PVC rather than in the container filesystem. Left
  alone, the file grows until the volume fills.

**Prefer not setting `LOG_FILE_PATH` at all.** By default `stellar-core` logs to
stdout, which the container runtime captures and your cluster's log pipeline
handles — including rotation, retention, and shipping off the node. That is
strictly better than a file on the ledger volume.

If you need both — for example an audit requirement that insists on a
node-local file alongside your normal pipeline — then you own the lifecycle:

- Point the log at its own `emptyDir` volume rather than the PVC, so it cannot
  consume ledger capacity. Add the volume and mount via `spec.volumes` and
  `spec.volumeMounts`; the mount must be writable, since the root filesystem is
  read-only.
- Ship the file out with a sidecar, and rotate it in that sidecar. There is no
  `logrotate` in the generated pod.
- Monitor the file's growth. [Proactive Disk Scaling](../proactive-disk-scaling.md)
  covers capacity alerts and PVC expansion.

!!! note "File logs are not picked up by the log aggregation recipes"

    The setups in the [Log Aggregation Guide](../log-aggregation-guide.md)
    (Promtail, Filebeat, Fluentd) read **container stdout** via the node log
    directory. They will not see a `stellar-core.log` written to the PVC. If you
    set `LOG_FILE_PATH`, you need a separate tailer.

## Related

- [CRD Reference](crd-reference.md) — every `StellarNode` field
- [Storage](storage.md) — PVC sizing, mounts, snapshot restore
- [Operator environment](operators.md) — operator process env vars, pod hardening, and env var override rules
- [Performance Tuning](../performance-tuning.md) — memory and cache sizing
- [Log Aggregation Guide](../log-aggregation-guide.md)
- [Proactive Disk Scaling](../proactive-disk-scaling.md)
- [mTLS Guide](../mtls-guide.md) — the known limitation on the mTLS config keys
