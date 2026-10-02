# Merkle Airdrop Claim Distributor

A pull-based token distribution contract for Stellar/Soroban. The full list of
`(address, allocation)` pairs is compiled **off-chain** into a single 32-byte Merkle root;
each recipient then proves their inclusion in that tree to pull their allocation **exactly
once**.

## Why this exists

Paying `N` recipients means `N` ledger writes and `N` token transfers, which is why a
token airdrop to 100 000+ accounts is economically impossible to run as a loop. Reducing
the whole distribution to one committed root flips the cost model:

| | push (transfer per recipient) | pull (this contract) |
|---|---|---|
| on-chain storage | `O(n)` | `O(1)` — one root |
| distributor transactions | `n` | `1` (initialize) + `1` (fund) |
| work per recipient | paid by the distributor | paid by the recipient, on claim |

The distributor never needs a transaction per recipient, and the recipient pays only for
their own proof.

## Features

- **One commitment for the whole distribution** — `initialize` stores a single root plus the
  allocation count, and nothing else that grows with the number of recipients.
- **SHA-256 proof verification** with a canonical, byte-exact tree format (below) that
  off-chain tooling can reproduce in any language.
- **Replay-proof claims.** A bitmap keyed by leaf index permanently flags claimed
  allocations, written *before* the token transfer. 128 allocations share one ledger entry,
  so a 1 048 576-recipient campaign needs at most 8 192 claim entries rather than one per
  recipient.
- **Exact payouts.** The amount and the index are hashed into the leaf, so a claimant can
  neither inflate a payout nor re-present an allocation under a different slot.
- **Bounded, predictable cost.** Proof depth is capped at 20 (2^20 = 1 048 576 padded
  leaves), so the worst-case cost of a claim is known before a campaign launches. Measured:
  **9 993 instructions per proof level**, 580 423 for a depth-20 claim — 0.15 % of the
  mainnet instruction ceiling.
- **Administration with teeth.** Root rotation is allowed only until the first claim; pausing
  only ever stops *new* claims; clawback requires a distribution that actually set a
  deadline.

## Modules

| File | Purpose |
|------|---------|
| `src/lib.rs` | Contract entrypoints: `initialize`, `set_merkle_root`, `claim`, `set_paused`, `clawback`, views |
| `src/claim.rs` | The whole trust surface: canonical leaf/node encodings, host hashing, proof verification, bitmap math |
| `src/offchain.rs` | Off-chain tree builder (native SHA-256). Never compiled into the contract — shared with the tests, benchmark and example by `#[path]` |
| `src/test.rs` | Contract behaviour: valid claims, replay, tampering, bitmap boundaries, admin surface, claim window |
| `tests/merkle_airdrop.rs` | End-to-end: an off-chain 100 000-recipient tree claimed on-chain |
| `benches/claim_bench.rs` | Instruction/fee cost at every proof depth 0..=20 |
| `examples/generate_tree.rs` | Operator tool: build a campaign's root and proofs |

## The canonical tree

```
leaf(i)    = SHA256( 0x00 ‖ i_be32 ‖ amount_be128 ‖ account_xdr )
node(a, b) = SHA256( 0x01 ‖ min(a, b) ‖ max(a, b) )
root       = the single digest left after folding adjacent pairs
```

- **Domain separation (`0x00` / `0x01`).** Without it the tree is vulnerable to the classic
  second-preimage attack, where a forged proof proves membership of an *intermediate* node
  and mints value nobody was allocated.
- **Sorted child pairs.** Folding `min ‖ max` removes the need for a direction bitmap
  alongside the proof: a proof is a plain `Vec<BytesN<32>>`, which is fewer bytes to decode
  out of calldata, one fewer branch per level, and — more importantly — removes an entire
  class of client bugs where the direction bits disagree with the sibling ordering.
- **The index and the amount are hashed in.** The claim flag is keyed by leaf index, so the
  index must be authenticated by the proof; and the contract pays exactly what the tree
  committed to.
- **`account_xdr` is the serialized `Address`** — 44 bytes for an account (`ScVal::Address` ‖
  `ScAddress::Account` ‖ `PublicKey::Ed25519` ‖ key), 40 for a contract address. Because it
  is the *trailing* field and everything before it is fixed width, the encoding is
  unambiguous, and hashing the address rather than its bare 32-byte payload means an account
  address and a contract address with the same payload can never collide.
- **Zero padding.** Allocations are padded with the all-zero digest up to a power of two, so
  the tree is perfectly balanced: every proof is `ceil(log2(count))` long and every claim
  costs the same. A padded slot cannot be claimed — it is not the image of any SHA-256 under
  this encoding.

These constants are asserted at compile time (`src/claim.rs`, `src/lib.rs`), and a golden
root is pinned in `tests/merkle_airdrop.rs`, so a change to the format cannot land by
accident.

## Building

```bash
cd contracts/merkle-airdrop
cargo build --target wasm32v1-none --release
```

`wasm32v1-none` — not `wasm32-unknown-unknown` — is the target this `soroban-sdk` version
supports on Rust 1.82+; the SDK's build script rejects the older target because it enables
WebAssembly features the environment does not implement. The result here is a 44 KB
`merkle_airdrop.wasm`.

## Testing

```bash
cd contracts/merkle-airdrop
cargo test                     # 38 behaviour tests + 10 integration tests
cargo test --bench claim_bench # per-depth instruction budget (also runs under `cargo bench`)
```

The integration suite builds a 100 000-recipient tree (131 072 padded leaves, depth 17) and
asserts valid claims, forged proofs, double claims, unallocated claimants, claim windows and
clawback against it. The benchmark additionally asserts that verification stays **linear** in
depth — the marginal cost of one level must not drift as the tree grows — and that the
deepest supported distribution fits inside a mainnet invocation with room to spare.

## Generating a campaign

```bash
cargo run --release --example generate_tree -- 100000
```

prints the root to commit with `initialize`, the total to fund, and (optionally) a sample
recipient's proof. Point `recipient_xdr` at real `Address::to_xdr` bytes to run a live
campaign.

## Operational notes

- **Fund the distributor, then announce it.** `initialize` deliberately does not move tokens,
  so a campaign can be staged and reviewed before the treasury moves. A claim for an unfunded
  distributor fails at the token transfer, not at verification.
- **`total_amount` is informational.** The hard cap on total outflow is the distributor's
  token balance; the contract's own invariant is "each index pays at most once".
- **The root freezes at the first claim.** Rotating it afterwards would reassign slots whose
  flags are already burned, stranding allocations the earlier root granted. A root that turns
  out to be wrong after claims begin is remedied by `clawback` (if a deadline was set) plus a
  fresh distributor.
- **Claim flags live in persistent storage**, which the host treats as effectively permanent:
  an expired entry is archived and restored rather than lost, so a lapsed TTL can never be
  observed as "not yet claimed". Each write also extends the entry's TTL so that an
  in-flight campaign never makes a claimant pay for a footprint restore.
- **Set a `claim_deadline` if you intend to sweep the remainder.** Without one, claims never
  expire and `clawback` is permanently disabled.

## API

```rust
initialize(admin, token, merkle_root, leaf_count, total_amount, claim_deadline)
set_merkle_root(caller, merkle_root, leaf_count, total_amount)   // before the first claim only
claim(claimant, index, amount, proof) -> ()                      // proof: Vec<BytesN<32>>
set_paused(caller, paused)
clawback(caller, recipient) -> i128                              // after the deadline only

merkle_root() -> BytesN<32>          leaf_count() -> u32            admin() -> Address
required_proof_depth() -> u32        total_allocated() -> i128      token() -> Address
claim_deadline() -> u64              progress() -> (i128, u32)      is_paused() -> bool
is_claimed(index) -> bool
```

Events (typed `#[contractevent]`): `init`, `root`, `claim` (topic: claimant), `paused`,
`clawback`.
