# ZKP Verifier for Private Transfers

> **Contract:** `contracts/zk-verifier`  
> **Issue:** [#301](https://github.com/agnesnaomiolim-cloud/Stellar-K8s/issues/301)  
> **Status:** Implemented  
> **Complexity:** High (200 Points)

---

## Overview

The ZKP Verifier is a Soroban smart contract that implements a **Zero-Knowledge Proof (ZKP) verifier** for privacy-preserving transfers on the Stellar network.

It enables users to prove the validity of a hidden transaction ("I have enough balance, and I haven't already spent this note") **without revealing the actual amounts, addresses, or any other private data** to the public ledger.

### Supported Proof Systems

| System   | Proof Size | Verify Cost (CPU) | Trusted Setup? | Notes                  |
|----------|-----------|-------------------|----------------|------------------------|
| Groth16  | 3 G1 + 1 G2 ≈ 192 bytes | ~19.7 M instr | Yes (per-circuit) | Industry standard, smallest proofs |
| PLONK    | 13 commitments + 6 evals ≈ ~640 bytes | ~8.7 M instr | Universal (updatable) | More flexible, no per-circuit ceremony |

---

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                        ZkVerifierContract                           │
│                                                                      │
│  ┌─────────────────┐  ┌─────────────────────┐  ┌────────────────┐  │
│  │  Shielded Pool  │  │  Nullifier Registry  │  │  Verifying Key │  │
│  │  (note cmts)    │  │  (Persistent Store)  │  │  (Instance)    │  │
│  └────────┬────────┘  └──────────┬──────────┘  └───────┬────────┘  │
│           │                       │                      │            │
│           └────────────┬──────────┘                      │            │
│                        ▼                                  ▼            │
│              verify_and_transfer()              Groth16 / PLONK       │
│                        │                        verify_*()            │
│                        │                                               │
│          ┌─────────────▼────────────────────────────────────────────┐ │
│          │  1. Check Merkle root is known                           │ │
│          │  2. Check nullifier not spent (BEFORE pairing)           │ │
│          │  3. Verify ZKP (Groth16 or PLONK)                        │ │
│          │  4. Spend nullifier in Persistent Storage                │ │
│          │  5. Emit masked transfer event                           │ │
│          └──────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────┘
```

### Modules

| File | Purpose |
|------|---------|
| `src/lib.rs` | Contract entry points: `init`, `deposit`, `verify_and_transfer`, `verify_groth16`, `verify_plonk`, admin functions |
| `src/types.rs` | Soroban `#[contracttype]` definitions: `G1Point`, `G2Point`, `Groth16Proof`, `PlonkProof`, `PublicInputs`, `NoteCommitment`, `AnyProof`, `VerifyResult` |
| `src/errors.rs` | `#[contracterror]` enum `ZkError` with stable numeric codes |
| `src/groth16.rs` | **Groth16 verifier** — BN254 field validation, pairing check, public-input MSM |
| `src/plonk.rs` | **PLONK verifier** — KZG opening check, Fiat-Shamir transcript, polynomial eval validation |
| `src/pool.rs` | **Shielded pool** — note commitment insertion, rolling Merkle root, known-root registry |
| `src/nullifier.rs` | **Nullifier registry** — Persistent Storage anti-replay protection with TTL management |
| `src/gas_profile.rs` | CPU instruction budget constants and `BudgetTracker` |
| `src/pairing.rs` | Re-export shim for `G1Point` / `G2Point`; documents future host-function pairing interface |

---

## Gas Profiling Report

### Soroban Resource Limits (Protocol 21)

| Resource | Per-Transaction Ceiling |
|----------|------------------------|
| CPU instructions | **100 000 000** (1 × 10⁸) |
| Memory | 40 000 000 bytes (40 MB) |
| Ledger reads | 40 |
| Ledger writes | 25 |
| Events | 3 |

### Instruction Cost Model

#### BN254 Pairing (Miller Loop + Final Exponentiation)

The dominant cost in both Groth16 and PLONK is the bilinear pairing `e: G1 × G2 → GT`.

| Sub-operation | Cost model | Estimated instructions |
|---------------|-----------|------------------------|
| Fp2 multiplication | 2 Fp-muls | 1 680 |
| Fp12 multiplication (Miller loop body) | 9 Fp2-muls | 15 120 |
| Miller loop (65 iterations) | 65 × 15 120 | 982 800 |
| Final exponentiation | ~2 970 Fp12-muls | 2 500 000 |
| **Total per pairing** | | **3 482 800** |

These estimates use Soroban's synthetic instruction model where:
- 1 field multiplication (Fp) ≈ 840 instructions  
- Source: `stellar-contract-env` benchmark suite on BN254

#### G1 Scalar Multiplication

| Operation | Estimated instructions |
|-----------|------------------------|
| G1 scalar mul (256-bit) | 175 000 |
| G2 scalar mul (256-bit) | 350 000 |

### Groth16 Full Verification Cost

```
Groth16 = 4 pairings + (n+1) G1 scalar muls

4 × e(A,B)    = 4 × 3 482 800 = 13 931 200
33 × G1-mul   = 33 × 175 000  =  5 775 000
                               ─────────────
TOTAL                          ≈ 19 706 200 instr

Budget ceiling (10 % reserve)  = 90 000 000 instr
Safety headroom                = 70 293 800 instr  (78 % unused)
```

**Groth16 uses 19.7 % of the transaction ceiling.**

### PLONK Full Verification Cost

```
PLONK = 2 pairings + ~10 G1 muls + 6 transcript hashes

2 × pairing   = 2 × 3 482 800 =  6 965 600
10 × G1-mul   = 10 × 175 000  =  1 750 000
6 × SHA-256   = 6 × 2 000     =     12 000
                               ─────────────
TOTAL                          ≈  8 727 600 instr

Budget ceiling (10 % reserve)  = 90 000 000 instr
Safety headroom                = 81 272 400 instr  (90 % unused)
```

**PLONK uses 8.7 % of the transaction ceiling.** It is the preferred system when proving time is not the bottleneck.

### Cost Comparison Table

| Operation | Instructions | % of TX Ceiling | Remaining Headroom |
|-----------|-------------|-----------------|-------------------|
| Groth16 verify (32 public inputs) | 19 706 200 | 19.7 % | 80 293 800 |
| PLONK verify (32 public inputs) | 8 727 600 | 8.7 % | 91 272 400 |
| Nullifier write (Persistent) | 4 000 | < 0.01 % | — |
| Event emission | 1 500 | < 0.01 % | — |
| Nullifier double-spend check (reject path) | ~4 000 | < 0.01 % | — |

### Why We Stay Well Below the Ceiling

1. **Pre-flight rejection**: A spent nullifier is detected in ~4 000 instructions (one persistent storage read), before any pairing arithmetic begins. Double-spend attacks are extremely cheap to reject.

2. **Budget tracker**: `BudgetTracker` accumulates estimated costs and returns `ZkError::CpuBudgetExceeded` before expensive operations if the ceiling would be exceeded.

3. **Static assertions**: The `gas_profile.rs` module includes compile-time `assert!` guards that prevent deployment of a contract with unsafe cost estimates.

4. **PLONK preference**: PLONK is 2.3× cheaper than Groth16 for pairing-heavy operations, making it the recommended system for high-throughput deployments.

### On-chain Gas Profile Events

Every invocation emits a `zkp_gas` Soroban event:

```rust
// Event key
(symbol_short!("zkp_gas"), system_id: u32)  // 0=Groth16, 1=PLONK

// Event data
(consumed: u64, ceiling: u64, utilisation_pct: u64)
```

These events can be consumed by monitoring infrastructure to build real-time dashboards of proof verification costs.

---

## Contract API

### Initialisation

```rust
fn init(
    env: Env,
    admin: Address,
    vk_groth16: Option<Groth16VerifyingKey>,
    vk_plonk: Option<PlonkVerifyingKey>,
) -> Result<(), ZkError>
```

Called once after deployment. Sets the admin address and optionally stores verifying keys.

### Depositing Notes

```rust
fn deposit(
    env: Env,
    commitment: BytesN<32>,   // Pedersen commitment to (recipient, amount, asset, blinding)
    sender: Address,
) -> Result<NoteCommitment, ZkError>
```

Inserts a shielded note commitment into the on-chain pool. Updates the incremental Merkle root. Emits a `deposit` event with the leaf index.

### Proof Verification + Private Transfer

```rust
fn verify_and_transfer(
    env: Env,
    proof: AnyProof,          // Groth16Proof or PlonkProof
    public_inputs: PublicInputs,
    relayer: Address,
) -> Result<VerifyResult, ZkError>
```

The core privacy-preserving entry point. Executes atomically:
1. Validates the Merkle root is known.
2. Checks the nullifier has not been spent.
3. Verifies the ZKP.
4. Marks the nullifier as spent in Persistent Storage.
5. Emits `nullify` and `transfer` events (no private data on-chain).

### Standalone Verification

```rust
fn verify_groth16(env, vk, proof, public_inputs) -> Result<bool, ZkError>
fn verify_plonk(env, vk, proof, public_inputs) -> Result<bool, ZkError>
```

Read-only verification without state changes. Used for pre-validation by clients.

### Queries

```rust
fn is_nullifier_spent(env, hash) -> bool
fn nullifier_spent_at(env, hash) -> Option<u32>   // ledger sequence
fn get_commitment_count(env) -> u32
fn is_known_root(env, root) -> bool
```

---

## Storage Layout

| Key | Storage type | Value | Description |
|-----|-------------|-------|-------------|
| `"admin"` | Instance | `Address` | Contract admin |
| `"vk_g16"` | Instance | `Groth16VerifyingKey` | Groth16 circuit VK |
| `"vk_plonk"` | Instance | `PlonkVerifyingKey` | PLONK circuit VK |
| `"init"` | Instance | `bool` | Initialisation flag |
| `"pool_cnt"` | Instance | `u32` | Note commitment counter (pool.rs) |
| `("pool_root", root_bytes)` | Instance | `u32` | Known Merkle roots → ledger seq (pool.rs) |
| `("nf", hash_bytes)` | **Persistent** | `u32` | Spent nullifiers → ledger seq (nullifier.rs) |

**Key design decisions:**
- Nullifiers use **Persistent** storage because they must survive ledger expiry. A spent nullifier that is inadvertently cleaned up would allow replay attacks.
- Verifying keys use **Instance** storage for fast access and easy admin upgrades.
- Merkle roots use **Instance** storage; the set of valid roots is small and rotated by the contract itself on each deposit.

---

## Replay-Attack Prevention

The nullifier registry in Persistent Storage provides unconditional replay protection:

```
Attacker submits previously-verified proof:
  → is_spent(nullifier_hash) = true   (cost: ~4 000 instr)
  → return ZkError::NullifierAlreadySpent
  → NO pairing arithmetic executed
  → attack cost: ~4 000 instructions + 1 read fee
```

The nullifier check is always performed **before** proof verification, ensuring that replay attacks are rejected at minimal cost.

---

## TTL Management

Persistent storage entries in Soroban have TTLs that can expire. The contract provides:

1. **Auto-extension on `spend`**: Every nullifier is immediately extended to ~3 years (`18_460_800` ledgers at 5 s/ledger) on first write.

2. **`extend_nullifier_ttl` function**: Permissionless TTL bump that can be called by anyone (or a cron-like TTL bumper service) to ensure nullifiers never expire.

3. **TTL bumper integration**: Use the existing `contracts/ttl-bumper` contract or the `stellar-operator prune-archive` utility.

---

## Off-Chain Integration

### Generating Proofs

Use a ZKP circuit compiler to generate proofs off-chain:

```bash
# gnark (Go)
cd circuits/private-transfer
go run main.go prove --circuit private_transfer.r1cs \
    --pk proving_key.bin \
    --input witness.json \
    --output proof.bin

# arkworks (Rust)
cargo run --example prove -- \
    --circuit private_transfer \
    --witness witness.json \
    --output proof.json
```

### Submitting to the Contract

```javascript
// Stellar JS SDK
import { Contract, xdr } from '@stellar/stellar-sdk';

const contract = new Contract(contractId);

// Build the invoke transaction
const tx = await contract.call('verify_and_transfer', [
    proofArg,        // AnyProof::Groth16(...)
    publicInputsArg, // PublicInputs { merkle_root, nullifier_hash, ... }
    relayerArg,      // Address
]);

const result = await server.simulateTransaction(tx);
// Check result.cost for actual on-chain instruction count
```

---

## E2E Testing

Full end-to-end tests (generate real proofs, verify on-chain) require:

1. A circuit compiler (gnark or arkworks) for proof generation.
2. A trusted setup ceremony (Groth16) or universal SRS (PLONK).
3. A local Soroban sandbox (`stellar-cli contract invoke`).

```bash
# Start local sandbox
stellar contract deploy --wasm target/wasm32-unknown-unknown/release/zk_verifier.wasm

# Run integration flow
stellar contract invoke \
    --id <contract-id> \
    --fn verify_and_transfer \
    -- \
    --proof '{"groth16": {...}}' \
    --public_inputs '{"merkle_root": "..."}' \
    --relayer <address>
```

---

## Security Considerations

| Threat | Mitigation |
|--------|-----------|
| Replay attack (double-spend) | Nullifier registry in Persistent Storage; pre-checked before pairings |
| Invalid proof forgery | Full structural validation + pairing check |
| Budget exhaustion / DoS | Pre-flight `BudgetTracker`; rejects before pairings if over budget |
| Admin key compromise | Admin can only update VKs; cannot drain funds or modify nullifiers |
| Expired nullifiers | Auto-TTL extension + permissionless TTL bumper |
| Unknown Merkle root | Contract maintains set of known roots; rejects stale roots |
| Zero nullifier sentinel | Explicitly rejected (`ZkError::NullifierIsZero`) |

---

## References

- Groth16: https://eprint.iacr.org/2016/260
- PLONK: https://eprint.iacr.org/2019/953
- Soroban Resource Limits (Protocol 21 / CAP-0046-10): https://developers.stellar.org/docs/learn/smart-contract-internals/gas-and-fees
- BN254 EIP-196/197: https://eips.ethereum.org/EIPS/eip-197
- Tornado Cash (nullifier pattern inspiration): https://github.com/tornadocash/tornado-core
