# Immutable Audit Ledger (tamper-evident hash chain)

> Issue #1958. Code: `src/audit_ledger/`, binaries `stellar-audit-sink` and
> `stellar-audit-verify`. This is separate from the operator's in-memory
> `audit_chain` module (operator-internal events); the ledger records **Kubernetes
> API server** mutating events.

## Why out-of-process

A compromised API server (or operator) can rewrite anything it logs in-process.
The ledger is therefore fed through the API server's **audit webhook backend** into
a separate pod (`stellar-audit-sink`) in its own namespace. Every entry commits to
its predecessor with SHA-256, so any later deletion, modification, insertion or
reordering is detectable by anyone holding the archive, with a single static binary.

```text
kube-apiserver ──audit webhook (batch)──▶ stellar-audit-sink ──append + fsync──▶ ledger.ndjson (PVC)
                                              │  /metrics                           │
                                              ▼                                     ▼
                                      Prometheus alerts              stellar-audit-verify (offline)
```

## Hash definition (version 1, normative)

`||` is concatenation, `be64` an 8-byte big-endian integer, hashes are raw 32-byte SHA-256.

```text
genesis      = SHA256("stellar-k8s/audit-ledger/v1/genesis\0" || chain_id_utf8)
content_hash = SHA256("stellar-k8s/audit-ledger/v1/content\0" || canonical_json(event))
entry_hash   = SHA256("stellar-k8s/audit-ledger/v1/entry\0"   || be64(seq) || prev_hash || content_hash)
```

* Entry `seq = 0` has `prev_hash = genesis`; each later entry has `seq = prev.seq + 1` and
  `prev_hash = prev.entry_hash`.
* Ordering is defined **only** by `seq`; timestamps in the event are informational.
* `canonical_json`: no whitespace; object keys sorted by UTF-8 bytes; arrays keep order;
  integers only (floats are rejected); strings escape only `"` `\` and C0 controls
  (`\b \t \n \f \r`, others as lowercase `\u00xx`); everything else is raw UTF-8.
* Known-answer vector (`chain_id = "prod"`, event `{"b":1,"a":2}`, seq 0):
  genesis `919a48ce11415cb2b2e4d466785b26178a045f9e30c51250212afb23f9c6ced9`,
  entry hash `cb39fa7a3f347c4909599dd5855a89691581cee60dd7beedd6641bd3a82de7cc`.

`content_hash` is separate from `entry_hash` so verification hashes event bodies on all
cores and runs only the 104-byte link hash sequentially.

## Archive format

UTF-8 NDJSON. Entry lines are strict: unknown fields, non-lowercase hex and torn final lines
are rejected (unauthenticated extra data cannot be smuggled in).

```text
{"type":"header","format":"stellar-audit-ledger/v1","chainId":"prod","genesisHash":"<hex>"}
{"seq":0,"prevHash":"<genesis>","hash":"<hex>","event":{...}}
{"seq":1,"prevHash":"<hash of 0>","hash":"<hex>","event":{...}}
{"type":"footer","count":2,"headSeq":1,"headHash":"<hex>"}     # written by `export`
```

A header may also carry `startSeq` + `startPrevHash` for a partial segment.

### Stored event fields

Only mutating verbs (`create update patch delete deletecollection`) are kept:
`auditID, stage, verb, level, requestURI (query string removed), user{username,groups},
objectRef{apiGroup,apiVersion,resource,subresource,namespace,name}, responseStatus.code,
requestReceivedTimestamp, stageTimestamp`. **Request/response bodies, annotations and
`user.extra` are never stored**; `sourceIPs` only with `--include-source-ips`.

## Delivery semantics

| Concern | Behaviour |
|---|---|
| Durability | `200` only after the batch is `fsync`ed. |
| Backpressure | Bounded queue; when full the sink replies `503` + `Retry-After`, the apiserver retries. |
| Duplicates | Same `auditID`+`stage` inside the dedupe window (default 200 000, rebuilt from the ledger tail on restart) is acknowledged but not re-appended. |
| Ordering | One writer assigns `seq` in arrival order; group commit merges concurrent requests into one fsync. |
| Failures | Failed batches are rolled back (uncommitted bytes only) and answered `500`; `write_failures_total` increments, `sink_up` goes to 0. |
| Invalid events | Counted in `events_invalid_total` and logged by `auditID`; never silently discarded. |
| Crash mid-write | A torn, never-acknowledged final line is discarded on restart; committed data is never touched. |

## Running it

```bash
kubectl apply -f config/audit-ledger/sink-deployment.yaml
# control plane flags: see config/audit-ledger/apiserver-webhook-kubeconfig.yaml
# audit policy:        config/audit-ledger/audit-policy.yaml
kubectl apply -f monitoring/audit-ledger-alerts.yaml
```

Use `--audit-webhook-mode=batch` (asynchronous). In `blocking` mode every API request waits for
the sink and the < 2 % overhead target does not apply.

### Append-only storage

The pipeline never opens the ledger with anything but `O_APPEND` and has no API to rewrite committed
bytes, but a process cannot protect a file from its own OS user. Enforce immutability below it:

* dedicated namespace/service account with no API permissions (shipped manifest);
* only the sink pod mounts the PVC read-write; verifiers get snapshots or read-only mounts;
* `chattr +a` on the ledger file (node-level), or a storage class with snapshots/retention;
* ship sealed archives (`stellar-audit-verify export`) to object storage with **Object Lock** (S3 compliance mode);
* **pin the head hash out-of-band** (e.g. record `headHash` from each export in a separate system) and
  verify with `--expect-head`; otherwise an attacker with full write access could truncate the tail and
  nothing inside the file can prove it.

## Verifying offline

```bash
stellar-audit-verify verify ledger.ndjson                 # exit 0 valid, 1 violation, 2 error
stellar-audit-verify verify ledger.ndjson --json --expect-head <hex> --expect-chain-id prod
stellar-audit-verify export ledger.ndjson sealed-2026-11-22.ndjson   # verified copy + footer
```

No cluster, network, keys or producer state are needed. Violations report the **exact sequence number**:

| `kind` | Meaning | `seq` |
|---|---|---|
| `missing_entries` | events deleted | first missing seq (`missingTo` = last) |
| `modified_entry` | event body/seq/hash altered | the altered entry |
| `predecessor_mismatch` | next entry commits to a different predecessor (attacker re-hashed a forged entry) | the replaced entry |
| `reordered_entry` | entry out of order / replayed | the seq that should have appeared |
| `duplicate_entry` | same entry twice | the duplicated seq |
| `genesis_mismatch` | wrong chain start | 0 |
| `footer_mismatch` / `head_mismatch` / `truncated` | tail removed | first seq that is missing |
| `malformed` / `header_invalid` | syntax, torn line, unknown field, wrong chain | – |

`scripts/audit-ledger-redteam.sh` runs the delete / modify / reorder drill end to end.

## Monitoring

`stellar-audit-sink` serves `/metrics` (Prometheus text) and `/healthz`.

| Metric | Meaning |
|---|---|
| `stellar_audit_ledger_events_{received,appended,duplicate,filtered,invalid}_total` | throughput and outcome counters |
| `stellar_audit_ledger_write_failures_total`, `_requests_backpressured_total` | delivery failures / backpressure |
| `stellar_audit_ledger_sink_up` | store availability |
| `stellar_audit_ledger_processing_latency_seconds` | receive → durable-ack histogram |
| `stellar_audit_ledger_chain_valid`, `_chain_gaps_total`, `_verification_failures_total` | integrity (latched) |
| `stellar_audit_ledger_verified_sequence`, `_head_sequence`, `_last_scan_timestamp_seconds` | progress / liveness |

**Gap alert latency.** The monitor verifies only newly appended bytes every `--scan-interval-secs`
(10 s) and re-checks that the last verified line is unchanged, which also catches deletions or
shifts earlier in the file. Worst case: 10 s scan + 15 s scrape + 15 s rule evaluation ≈ 40 s
(`AuditLedgerChainBroken`, `for: 0m`). A same-length in-place edit deep in history is invisible to the
incremental check by design; the monitor runs a full parallel verification every `--full-verify-every`
scans (default 360 ≈ 1 h).

## Performance

See the PR description for measured numbers. Reproduce:

```bash
cargo test --release --lib audit_ledger_scale -- --ignored --nocapture      # AUDIT_SCALE_N, AUDIT_SCALE_DIR
# or with the binaries:
stellar-audit-verify generate --out /data/10m.ndjson --count 10000000
time stellar-audit-verify verify /data/10m.ndjson
```

## Incident response

1. `AuditLedgerChainBroken` fires → **snapshot the volume first**; do not restart the sink.
2. `stellar-audit-verify verify <snapshot> --json` → note `kind` and `seq`.
3. Compare against the last pinned head / sealed archives to bound the damage window.
4. The sink keeps appending after a violation (new events are still recorded); the monitor stays latched
   until it is restarted after investigation.

## Known limitations

* Single writer (one sink replica); HA needs an external consensus/lock and is out of scope.
* The file store is the only backend; sealed-segment upload to S3 Object Lock is documented but not automated.
* Tail truncation is only provable against an externally pinned head or footer.
* Measuring apiserver throughput overhead requires a real control plane; see the PR for what was and was not measured.
