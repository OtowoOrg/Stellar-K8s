# mpt-verifier

A Soroban contract that verifies Ethereum Merkle Patricia Trie (MPT) state
proofs, so a bridge can validate a cross-chain deposit without trusting an
off-chain oracle.

## What it does

Given the RLP-encoded trie nodes Ethereum's `eth_getProof` returns, the contract
checks that they hash from a caller-supplied state root to a leaf holding the
expected value:

1. **RLP decode** — every node is decoded under strict canonical-form rules and
   a byte budget.
2. **Keccak-256** — each node is re-hashed, binding it to its place in the trie.
3. **Traverse** — the path is walked branch by branch, following 32-byte child
   hashes or short inline children, to a leaf whose hex-prefix path matches the
   requested key exactly.
4. **Decode the value** — the account leaf yields nonce, balance, storage root
   and code hash; a storage leaf yields the slot's integer value.

A storage-slot proof is checked against the `storageRoot` inside the *verified
account leaf*, so the two proofs bind to each other: a valid slot proof for a
different account is useless.

## Contract interface

```rust
MptVerifier::verify_evm_state(
    env: Env,
    state_root: BytesN<32>,      // the root the caller expects
    account_address: BytesN<20>, // whose state is being claimed
    framed_nodes: Bytes,         // eth_getProof nodes, length-prefixed
    framed_slots: Bytes,         // optional slot proofs, same framing
) -> Result<VerifiedEvmState, MptError>
```

The result carries the account's nonce, balance, storage root and code hash,
plus the value of every requested storage slot.

**Why proofs are length-prefixed.** The Soroban ABI cannot carry a
variable-length collection in a contract type, so the node list arrives as one
blob framed as `[len: u32 big-endian][bytes]…`. `frame_nodes` and `frame_slot`
build this format; every length is validated before use, so a malformed frame
is rejected rather than read out of bounds.

## Security properties

Proof bytes are attacker-controlled, so the work is bounded on every axis that
could otherwise be exploited:

| Bound | Default | Stops |
| --- | --- | --- |
| `trie::Limits::max_depth` | 256 nibbles | maliciously long extension chains |
| `trie::Limits::max_nodes` | 512 | oversized node sets |
| `trie::Limits::max_node_bytes` | 8 KiB | a single huge node |
| `trie::Limits::max_total_bytes` | 256 KiB | aggregate memory use |
| `rlp::Limits::max_depth` | 4 | deeply nested RLP |
| `rlp::Limits::max_item_bytes` | 8 KiB | a single huge RLP string |

Additional hardening:

- **Iterative traversal.** No recursion, so a deep proof cannot overflow the
  Wasm stack.
- **Canonical RLP only.** Short forms are required when the payload allows and
  length fields may not carry leading zeros. Without this, the same node would
  have several byte-distinct encodings with different hashes, and a verifier
  hashing attacker-supplied bytes could be shown one encoding and made to
  reconstruct another.
- **Nothing is trusted implicitly.** The state root is the caller's commitment,
  so a proof that is internally consistent but offered against a different root
  is refused. Each node is bound to the trie by its hash, so flipping one bit
  anywhere invalidates verification.
- **Non-inclusion is distinguished.** A proof terminating on a different key, or
  on an empty branch slot, is reported as absence rather than being mistaken
  for presence.

## Keccak-256

Keccak-256 is Ethereum's pre-NIST variant (padding `0x01`), **not** SHA3-256
(padding `0x06`). `tests/keccak_audit.rs` pins the empty-string digest, a
standard test vector, and the empty-trie root, and asserts the two padding
variants differ. The digests were additionally reproduced with an independent
Keccak-256 implementation written from the specification, agreeing with
`tiny-keccak` on every vector including inputs spanning multiple rate blocks.

## Validation against real mainnet data

`tests/fixtures/mainnet_weth_state_proof.json` is a real proof captured from
Ethereum mainnet via `eth_getProof` and `eth_getBlockByNumber`, recording the
block hash, state root, a 9-node account proof for the WETH contract and a
9-node storage proof for a WETH balance. `tests/mainnet_state_proof.rs` drives
it and asserts the decoded nonce, balance, code hash and slot value all match
what the node reported.

The tests also assert the security properties against that real proof: every
single-byte flip in every node is rejected, a wrong state root is rejected, a
truncated proof is rejected, and a different address never receives WETH's data.

Re-capture instructions are in the fixture's `_comment` field.

## Build and test

```bash
cargo test                                  # unit, mainnet and Keccak tests
cargo clippy --all-targets                  # lints
cargo fmt --check                           # formatting
cargo build --features contract-wasm \
    --target wasm32v1-none --release         # deployable WASM
```

`contract-wasm` enables the `no_std` build. It is off by default because
`cargo test` links host binaries, which need `std` for an allocator and panic
handler.

The Soroban environment requires the `wasm32v1-none` target; building for
`wasm32-unknown-unknown` is rejected by `soroban-sdk`'s build script.

## Not covered

- The state root itself is assumed correct. Verifying it against a block header
  is a light-client or checkpoint problem, deliberately out of scope here.
- No live Stellar network or EVM was used. Verification is exercised through
  `soroban-sdk`'s test environment and against recorded mainnet data.
