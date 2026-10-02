# Stellar State Verifier

A Soroban oracle that lets other contracts check that a transaction was
applied on the Stellar network, and with which result. It is meant as a
building block for trustless bridges and layer-2 systems.

| Module | Role |
|--------|------|
| [`src/lib.rs`](src/lib.rs) | Contract: relayer checkpoints, ancestor proofs, results commitment, verification |
| [`src/xdr_parser.rs`](src/xdr_parser.rs) | Strict XDR decoder for `LedgerHeader`, `TransactionResultSet` and `TransactionEnvelope`, running inside WASM |
| [`src/merkle.rs`](src/merkle.rs) | Merkle commitment over `(tx_hash, result_code)` pairs and the verification loop |

## How verification works

```text
 tx hash ──Merkle proof──▶ results root ──sha256(ResultSet)──▶ txSetResultHash
                                                                   │
                                 ledger hash = sha256(LedgerHeader)◀┘
                                       │
              relayer checkpoint ──────┤ (SCP value signed by a trusted validator)
              or ancestor proof ───────┘ (hash-linked to a verified child)
```

1. **`checkpoint_ledger(relayer, header_xdr)`**: an authorised relayer
   submits a raw `LedgerHeader`. The contract decodes it and checks that the
   SCP value carries a valid ed25519 signature from a trusted validator.
   The signed payload is
   `networkID ‖ ENVELOPE_TYPE_SCPVALUE ‖ txSetHash ‖ closeTime`. The
   contract then records `sha256(header)` as the ledger hash.
2. **`prove_ancestor(header_xdr)`**: anyone can extend the verified set
   backwards, with no trust needed. A header is accepted when it hashes to
   the `previousLedgerHash` of a verified ledger.
3. **`commit_results(ledger_seq, results_xdr)`**: anyone can submit the
   ledger's `TransactionResultSet`, once per ledger. Stellar commits results
   linearly (`txSetResultHash = sha256(TransactionResultSet)`) rather than
   as a tree. The contract checks the hash against the header, decodes every
   `(tx_hash, result_code)` pair, and stores a Merkle root over them.
4. **`verify_transaction(ledger_seq, tx_hash, result_code, index, proof)`**
   runs an O(log n) Merkle proof. **`verify_envelope(...)`** also decodes the
   `TransactionEnvelope`, recomputes its network-bound hash, and returns the
   source, fee payer, memo, time bounds and operations. Payments, path
   payments, account creation and merges are returned in full.

Every failure panics with a `VerifierError`, so a calling contract reverts
atomically on a bad proof.

### Trust model

- **Relayers are the trust root for anchored headers.** The validator
  signature covers only `txSetHash` and `closeTime`, not the whole header.
  It proves the ledger value was nominated by a trusted validator on this
  network, but a malicious relayer could still pair a real signature with
  other forged header fields. Only authorise relayers you trust, and anchor
  headers that your own node has observed.
- **Ancestors and results are trustless.** Both are bound to an anchored
  header by SHA-256.
- **Forward extension is deliberately impossible.** Anyone can build a
  header that points at a given parent, so a child header proves nothing.

### Merkle commitment

```text
leaf = sha256(0x00 ‖ tx_hash ‖ result_code as big-endian i32)
node = sha256(0x01 ‖ left ‖ right)
```

- The tree has `ceil(log2(n))` levels and is padded to a power of two with
  zero leaves.
- The `0x00`/`0x01` prefixes (domain separation) stop a node from being
  passed off as a leaf.
- Proofs are the sibling hashes, bottom-up, concatenated into one `Bytes`
  value. The leaf index selects left or right at each level, so it is bound
  to the proof.
- `merkle::proof` and `xdr_parser::split_result_set` are the reference
  helpers for off-chain provers.

### What the envelope decoder supports

- **Envelopes:** v0, v1 and fee-bump.
- **Operations:** every classic operation except `RevokeSponsorship`.
- **Unsupported:** Soroban transactions. The decoder reports them as
  `UnsupportedTransaction`, but they can still be proven with
  `verify_transaction`, which needs only the hash.

## Gas profile

Measured on the release WASM in the Soroban VM (`make profile`), against
mainnet ledger 64701890 (protocol 28, 172 transactions, 37 KB of results):

| Entry point | CPU instructions | % of tx limit | Memory |
|-------------|-----------------:|--------------:|-------:|
| `checkpoint_ledger` | 1,156,412 | 1.16% | 1.3 MB |
| `prove_ancestor` | 674,305 | 0.67% | 1.3 MB |
| `commit_results` | 15,655,883 | 15.66% | 1.4 MB |
| `verify_transaction` | 688,683 | 0.69% | 1.2 MB |
| `verify_envelope` (payment) | 996,392 | 1.00% | 1.3 MB |
| `verify_envelope` (fee bump) | 1,022,827 | 1.02% | 1.3 MB |

**Worst case.** A result set is bounded by the 132 KB transaction-size
limit. A 130 KB set of real result pairs (583 transactions, depth 10)
commits in **52.9M instructions (52.9%)**. So any result set that fits in a
transaction also fits the CPU limit.

The tests fail if any entry point exceeds the network limits. They also
enforce regression ceilings of 2M instructions for `verify_transaction` and
4M for `verify_envelope`.

### Optimisations

- **Single copy into WASM memory.** Each input is copied once, and the
  decoder reads a plain slice, so there are no per-field host calls.
- **Skip-only readers.** Fields the verifier discards, such as offers,
  claim atoms and signatures, are skipped without being copied.
- **Proof loop.** The whole proof is copied with one host call. Each level
  then reuses one 65-byte stack buffer and makes exactly one `sha256` call.
- **Padding.** Empty subtrees are cached per level, so building the tree
  costs about n hashes, not 2^depth.
- **`opt-level = 2`.** This halves the CPU cost compared with `"z"`, for
  about 1 KB of extra WASM.

## Build & test

Requires the `wasm32v1-none` target (`rustup target add wasm32v1-none`).

```sh
make test      # build the WASM, then run unit tests + WASM gas profile
make profile   # print the gas table
make lint      # rustfmt check + clippy (host and wasm) with -D warnings
make fixture   # re-fetch the mainnet fixture from Horizon
```

`scripts/fetch_fixture.py` checks that the fixture it writes is internally
consistent:

- the header hashes to the published ledger hash;
- the parent header hashes to the header's `previousLedgerHash`;
- the rebuilt result set hashes to the header's `txSetResultHash`.

It also computes the expected Merkle root on its own. The tests check that
the contract produces the same root.
