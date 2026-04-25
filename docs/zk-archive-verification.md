# ZK Archive Verification

Stellar-K8s can verify the integrity and completeness of encrypted history archives without requiring the operator to possess the decryption key. This is done through a **signed manifest + hash-chain** scheme that satisfies the "no-knowledge" property: the verifier reads only a small JSON file, never the encrypted checkpoint data.

---

## How It Works

When history archives are stored in cold storage in encrypted form, a publisher generates an `archive-manifest.json` alongside the ciphertext. The manifest records:

1. **Every checkpoint** present in the archive — its ledger sequence number and the SHA-256 hash of the encrypted file.
2. **A forward hash chain** — each entry's `prev_hash` field is the SHA-256 of the preceding entry's `sequence:file_hash` string, binding all entries into a tamper-evident chain.
3. **An Ed25519 signature** over the manifest body, so the verifier can confirm the manifest was not altered after publication.

The Stellar-K8s operator fetches `{archive_url}/.well-known/archive-manifest.json` and runs three checks:

| Check | Passes when |
|-------|-------------|
| **Signature** | The Ed25519 signature covers the exact manifest body and matches the embedded (or configured) signer key |
| **Completeness** | Checkpoint sequence numbers form a contiguous series at 64-ledger intervals — no ranges are missing |
| **Hash chain** | Each entry's `prev_hash` equals `SHA-256("{prev_sequence}:{prev_file_hash}")` |

If **any check fails**, the Archive Health condition for that node is set to `False` and the failure is reported in the status conditions and Prometheus metrics.

Archives that have no `archive-manifest.json` (plain, unencrypted archives) are treated as passing — the check is skipped automatically.

---

## Manifest Format

Place a file at `{archive_url}/.well-known/archive-manifest.json` with the following structure:

```json
{
  "version": 1,
  "archive_url": "s3://my-bucket/stellar-history",
  "current_ledger": 1048575,
  "checkpoints": [
    {
      "sequence": 63,
      "file_hash": "a1b2c3d4...",
      "prev_hash": "0"
    },
    {
      "sequence": 127,
      "file_hash": "e5f6a7b8...",
      "prev_hash": "sha256(63:a1b2c3d4...)"
    },
    ...
  ],
  "signer_key": "hex-encoded-ed25519-public-key (32 bytes = 64 hex chars)",
  "signature": "hex-encoded-ed25519-signature (64 bytes = 128 hex chars)"
}
```

### Field reference

| Field | Type | Description |
|-------|------|-------------|
| `version` | integer | Always `1` for this format |
| `archive_url` | string | Canonical URL of the archive |
| `current_ledger` | integer | Latest ledger included in the archive |
| `checkpoints[].sequence` | integer | Ledger number (e.g. 63, 127, 191, …) |
| `checkpoints[].file_hash` | hex string | SHA-256 of the encrypted checkpoint file |
| `checkpoints[].prev_hash` | hex string | SHA-256 of `"{prev_sequence}:{prev_file_hash}"`; `"0"` for the first entry |
| `signer_key` | hex string | 32-byte Ed25519 public key |
| `signature` | hex string | 64-byte Ed25519 signature over the JSON body (all fields except `signer_key` and `signature`) |

---

## Generating a Manifest

The manifest must be produced by whoever holds the archive encryption key. Here is a minimal example using `openssl` and `jq`:

```bash
# 1. Generate an Ed25519 keypair (if you don't have one)
openssl genpkey -algorithm ed25519 -out archive-signing-key.pem
openssl pkey -in archive-signing-key.pem -pubout -out archive-signing-key-pub.pem

# 2. Build the manifest body (fill in your real values)
BODY=$(jq -n --argjson checkpoints "$(cat checkpoints.json)" '{
  "version": 1,
  "archive_url": "s3://my-bucket/stellar-history",
  "current_ledger": 1048575,
  "checkpoints": $checkpoints
}')

# 3. Sign the body
SIG_HEX=$(echo -n "$BODY" | openssl dgst -sha512 -sign archive-signing-key.pem | xxd -p -c 999)

# 4. Extract the public key as hex
PUB_HEX=$(openssl pkey -in archive-signing-key-pub.pem -pubin -noout -text 2>&1 | \
  grep -A1 "pub:" | tail -1 | tr -d ' \n:')

# 5. Assemble the full manifest
echo "$BODY" | jq --arg sig "$SIG_HEX" --arg key "$PUB_HEX" \
  '. + {signer_key: $key, signature: $sig}' > archive-manifest.json

# 6. Upload alongside the encrypted archive
aws s3 cp archive-manifest.json \
  s3://my-bucket/stellar-history/.well-known/archive-manifest.json
```

---

## Configuring Verification

### Specifying the expected signer key

To pin the operator to a specific signing key and reject manifests signed by any other key, set `archiveSignerKey` in your `StellarNode` spec:

```yaml
spec:
  validatorConfig:
    historyArchiveUrls:
      - "s3://my-bucket/stellar-history"
    archiveSignerKey: "aabbccddeeff..."  # 64-char hex Ed25519 public key
```

If `archiveSignerKey` is omitted, the operator still verifies the signature against whichever key is embedded in the manifest.

### Feature flag

ZK archive verification is governed by the `enable_archive_health` runtime feature flag (enabled by default):

```yaml
# stellar-operator-config ConfigMap
data:
  enable_archive_health: "true"
```

---

## Prometheus Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `stellar_zk_archive_signature_valid` | Gauge | namespace, name, node_type, network | `1` = valid signature, `0` = invalid or manifest absent |
| `stellar_zk_archive_chain_gaps_total` | Gauge | namespace, name, node_type, network | Number of gap ranges detected; `0` = complete archive |

### Example alert rules

```yaml
groups:
  - name: zk_archive
    rules:
      - alert: ZkArchiveSignatureInvalid
        expr: stellar_zk_archive_signature_valid == 0
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Archive manifest signature invalid for {{ $labels.name }}"
          description: "The archive manifest signature check failed. The archive may have been tampered with."

      - alert: ZkArchiveGapsDetected
        expr: stellar_zk_archive_chain_gaps_total > 0
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "Checkpoint gaps in archive {{ $labels.name }}"
          description: "{{ $value }} gap range(s) detected in the ZK archive hash chain."
```

---

## Troubleshooting

**`stellar_zk_archive_signature_valid` is 0**

- The manifest signature does not match the signer key embedded in the manifest (or the configured `archiveSignerKey`).
- Check that the manifest was signed with the correct private key and uploaded without modification.
- Verify the `signer_key` field contains the hex-encoded *public* key, not the private key.

**`stellar_zk_archive_chain_gaps_total` is > 0**

- One or more checkpoint files are missing from the archive — either they were never uploaded or were deleted.
- Check the operator logs for the specific ledger ranges that are missing.
- Re-run the archive ingestion process for the missing ledger ranges.

**Archive Health condition shows `ZkVerificationError`**

- An HTTP error occurred while fetching the manifest. Check that the archive URL is reachable and the `.well-known/archive-manifest.json` path is publicly readable.

**Verification is skipped ("No manifest present")**

- The archive at that URL has no `archive-manifest.json`. This is expected for plain unencrypted archives. If the archive is encrypted, ensure the manifest was uploaded to the correct path.
