//! # zk-verifier
//!
//! A Soroban smart contract implementing a **Zero-Knowledge Proof verifier**
//! for privacy-preserving transfers on the Stellar network.
//!
//! ## Architecture
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────────────┐
//!  │                      ZkVerifierContract                         │
//!  │                                                                  │
//!  │  ┌──────────────┐  ┌────────────────────┐  ┌─────────────────┐ │
//!  │  │  Shielded    │  │  Nullifier         │  │  Verifying Key  │ │
//!  │  │  Pool        │  │  Registry          │  │  Registry       │ │
//!  │  │  (pool.rs)   │  │  (nullifier.rs)    │  │  (Instance      │ │
//!  │  │              │  │  Persistent        │  │   Storage)      │ │
//!  │  └──────────────┘  └────────────────────┘  └─────────────────┘ │
//!  │                                                                  │
//!  │  ┌──────────────┐  ┌────────────────────┐                      │
//!  │  │  Groth16     │  │  PLONK             │                      │
//!  │  │  (groth16.rs)│  │  (plonk.rs)        │                      │
//!  │  └──────────────┘  └────────────────────┘                      │
//!  └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key Entry Points
//!
//! | Function                  | Description                                    |
//! |---------------------------|------------------------------------------------|
//! | `init`                    | One-time initialisation, sets admin + VKs      |
//! | `deposit`                 | Submit a note commitment to the shielded pool  |
//! | `verify_and_transfer`     | Verify ZKP + spend nullifier + emit transfer   |
//! | `verify_groth16`          | Standalone Groth16 verification (no side-fx)   |
//! | `verify_plonk`            | Standalone PLONK verification (no side-fx)     |
//! | `is_nullifier_spent`      | Query nullifier registry                       |
//! | `get_commitment_count`    | Query commitment tree size                     |
//! | `set_verifying_key_g16`   | Admin: update Groth16 VK                       |
//! | `set_verifying_key_plonk` | Admin: update PLONK VK                         |
//! | `extend_nullifier_ttl`    | Permissionless TTL bumper for nullifiers        |
//!
//! ## Replay-Attack Prevention
//!
//! Every accepted proof causes a `nullifier_hash` to be written to Persistent
//! storage.  If the same nullifier is submitted a second time, the contract
//! returns `ZkError::NullifierAlreadySpent` before touching any pairing
//! arithmetic – making double-spend attempts very cheap to reject.
//!
//! ## Gas Profiling
//!
//! Every invocation emits a `zkp_gas` event with the estimated instruction
//! count.  See `docs/zk-verifier.md` for the full profiling report.

#![no_std]

pub mod errors;
pub mod gas_profile;
pub mod groth16;
pub mod nullifier;
pub mod pairing;
pub mod plonk;
pub mod pool;
pub mod types;

use soroban_sdk::{
    contract, contractimpl, symbol_short, Address, BytesN, Env, Symbol, Vec,
};

use errors::ZkError;
use gas_profile::{emit_profile_event, BudgetTracker, STORAGE_ENTRY_INSTR};
use types::{
    AnyProof, Groth16Proof, Groth16VerifyingKey, NoteCommitment, NullifierHash, PlonkProof,
    PlonkVerifyingKey, ProofSystem, PublicInputs, VerifyResult,
};

// ─── Storage Keys ──────────────────────────────────────────────────────────

/// Contract admin (set during `init`, can update verifying keys).
const KEY_ADMIN: Symbol = symbol_short!("admin");
/// Groth16 verifying key stored in Instance storage.
const KEY_VK_G16: Symbol = symbol_short!("vk_g16");
/// PLONK verifying key stored in Instance storage.
const KEY_VK_PLONK: Symbol = symbol_short!("vk_plonk");
/// Whether the contract has been initialised.
const KEY_INIT: Symbol = symbol_short!("init");

// ─── Events ────────────────────────────────────────────────────────────────

/// Event topic emitted when a note commitment is deposited.
const EVT_DEPOSIT: Symbol = symbol_short!("deposit");
/// Event topic emitted when a proof is verified and a transfer masked.
const EVT_TRANSFER: Symbol = symbol_short!("transfer");
/// Event topic emitted when a nullifier is spent.
const EVT_NULLIFIER: Symbol = symbol_short!("nullify");

// ─── Internal Helpers ──────────────────────────────────────────────────────

fn assert_initialised(env: &Env) -> Result<(), ZkError> {
    if !env.storage().instance().has(&KEY_INIT) {
        return Err(ZkError::NotInitialised);
    }
    Ok(())
}

fn assert_admin(env: &Env, caller: &Address) -> Result<(), ZkError> {
    assert_initialised(env)?;
    let admin: Address = env
        .storage()
        .instance()
        .get(&KEY_ADMIN)
        .ok_or(ZkError::NotInitialised)?;
    if *caller != admin {
        return Err(ZkError::Unauthorised);
    }
    Ok(())
}

// ─── Contract ──────────────────────────────────────────────────────────────

/// The ZKP Verifier / Shielded Pool contract.
///
/// Manages:
/// * A commitment tree whose Merkle root is tracked on-chain (`pool` module).
/// * A nullifier registry in Persistent storage (`nullifier` module).
/// * Groth16 and PLONK verifying keys in Instance storage.
/// * Privacy-preserving transfer state-transition logic.
#[contract]
pub struct ZkVerifierContract;

#[contractimpl]
impl ZkVerifierContract {
    // ── Initialisation ────────────────────────────────────────────────

    /// Initialise the contract.  Can only be called once.
    ///
    /// Sets the admin address, initialises the shielded pool, and optionally
    /// stores the initial Groth16 and PLONK verifying keys.
    pub fn init(
        env: Env,
        admin: Address,
        vk_groth16: Option<Groth16VerifyingKey>,
        vk_plonk: Option<PlonkVerifyingKey>,
    ) -> Result<(), ZkError> {
        if env.storage().instance().has(&KEY_INIT) {
            return Err(ZkError::AlreadyInitialised);
        }
        admin.require_auth();

        env.storage().instance().set(&KEY_ADMIN, &admin);
        env.storage().instance().set(&KEY_INIT, &true);

        // Initialise the shielded pool commitment counter.
        pool::init_pool(&env);

        if let Some(vk) = vk_groth16 {
            env.storage().instance().set(&KEY_VK_G16, &vk);
        }
        if let Some(vk) = vk_plonk {
            env.storage().instance().set(&KEY_VK_PLONK, &vk);
        }

        Ok(())
    }

    // ── Shielded Pool ─────────────────────────────────────────────────

    /// Deposit a shielded note commitment into the pool.
    ///
    /// The commitment should be a Pedersen commitment to (recipient, amount,
    /// asset, blinding_factor) computed off-chain by the sender.
    ///
    /// The Merkle root is updated after each insertion via `pool::insert_commitment`.
    ///
    /// # Errors
    ///
    /// * [`ZkError::NotInitialised`] – contract not yet set up.
    /// * [`ZkError::InvalidNoteCommitment`] – commitment is the zero value.
    /// * [`ZkError::CommitmentTreeFull`] – max leaf count reached.
    pub fn deposit(
        env: Env,
        commitment: BytesN<32>,
        sender: Address,
    ) -> Result<NoteCommitment, ZkError> {
        assert_initialised(&env)?;
        sender.require_auth();

        // Delegate all pool insertion logic to the pool module:
        // zero-check, capacity-check, counter increment, root computation + registration.
        let note = pool::insert_commitment(&env, &commitment)?;

        // Emit deposit event so off-chain indexers can track insertions.
        env.events().publish(
            (EVT_DEPOSIT, note.leaf_index),
            (commitment, note.inserted_at),
        );

        Ok(note)
    }

    // ── Proof Verification ────────────────────────────────────────────

    /// Verify a ZKP proof and execute a private transfer.
    ///
    /// This is the core privacy-preserving entry point.  It:
    /// 1. Checks the Merkle root is known (via `pool::is_known_root`).
    /// 2. Checks the nullifier is not spent (anti-replay, via `nullifier::is_spent`).
    /// 3. Verifies the proof (Groth16 → `groth16::verify_groth16`,
    ///    PLONK → `plonk::verify_plonk`).
    /// 4. Marks the nullifier as spent (via `nullifier::spend`).
    /// 5. Emits masked transfer events (no amounts or addresses on-chain).
    ///
    /// # Errors
    ///
    /// All [`ZkError`] variants.
    pub fn verify_and_transfer(
        env: Env,
        proof: AnyProof,
        public_inputs: PublicInputs,
        relayer: Address,
    ) -> Result<VerifyResult, ZkError> {
        assert_initialised(&env)?;
        relayer.require_auth();

        let mut tracker = BudgetTracker::new();
        tracker.add(STORAGE_ENTRY_INSTR); // init overhead

        // ── Step 1: Validate the Merkle root ────────────────────────
        if !pool::is_known_root(&env, &public_inputs.merkle_root) {
            return Err(ZkError::UnknownMerkleRoot);
        }

        // ── Step 2: Nullifier pre-check (cheap, before pairings) ────
        if nullifier::is_spent(&env, &public_inputs.nullifier_hash) {
            return Err(ZkError::NullifierAlreadySpent);
        }

        // ── Step 3: Proof verification ───────────────────────────────
        let system = match &proof {
            AnyProof::Groth16(p) => {
                Self::do_verify_groth16(&env, p, &public_inputs, &mut tracker)?;
                ProofSystem::Groth16
            }
            AnyProof::Plonk(p) => {
                Self::do_verify_plonk(&env, p, &public_inputs, &mut tracker)?;
                ProofSystem::Plonk
            }
        };

        // ── Step 4: Spend nullifier ───────────────────────────────────
        nullifier::spend(&env, &public_inputs.nullifier_hash)?;

        // ── Step 5: Emit events ───────────────────────────────────────
        let system_id = match system {
            ProofSystem::Groth16 => 0u32,
            ProofSystem::Plonk => 1u32,
        };
        emit_profile_event(&env, system_id, &tracker);

        env.events().publish(
            (EVT_NULLIFIER,),
            (public_inputs.nullifier_hash.clone(),),
        );

        env.events().publish(
            (EVT_TRANSFER,),
            (
                public_inputs.asset_id.clone(),
                public_inputs.relayer_fee,
            ),
        );

        Ok(VerifyResult {
            nullifier_hash: public_inputs.nullifier_hash,
            merkle_root: public_inputs.merkle_root,
            proof_system: system,
            cpu_instructions: tracker.consumed,
        })
    }

    /// Standalone Groth16 proof verification (read-only, no state changes).
    ///
    /// Delegates to [`groth16::verify_groth16`].  Useful for clients that want
    /// to pre-verify a proof locally before broadcasting.
    pub fn verify_groth16(
        env: Env,
        vk: Groth16VerifyingKey,
        proof: Groth16Proof,
        public_inputs: Vec<BytesN<32>>,
    ) -> Result<bool, ZkError> {
        let mut tracker = BudgetTracker::new();
        groth16::verify_groth16(&env, &vk, &proof, &public_inputs, &mut tracker)?;
        emit_profile_event(&env, 0, &tracker);
        Ok(true)
    }

    /// Standalone PLONK proof verification (read-only, no state changes).
    ///
    /// Delegates to [`plonk::verify_plonk`].
    pub fn verify_plonk(
        env: Env,
        vk: PlonkVerifyingKey,
        proof: PlonkProof,
        public_inputs: PublicInputs,
    ) -> Result<bool, ZkError> {
        let mut tracker = BudgetTracker::new();
        plonk::verify_plonk(&env, &vk, &proof, &public_inputs, &mut tracker)?;
        emit_profile_event(&env, 1, &tracker);
        Ok(true)
    }

    // ── Queries ───────────────────────────────────────────────────────

    /// Check whether a nullifier has been spent.
    pub fn is_nullifier_spent(env: Env, nullifier_hash: NullifierHash) -> bool {
        nullifier::is_spent(&env, &nullifier_hash)
    }

    /// Return the ledger sequence at which a nullifier was spent, or `None`.
    pub fn nullifier_spent_at(env: Env, nullifier_hash: NullifierHash) -> Option<u32> {
        nullifier::spent_at(&env, &nullifier_hash)
    }

    /// Return the number of note commitments deposited so far.
    pub fn get_commitment_count(env: Env) -> u32 {
        pool::commitment_count(&env)
    }

    /// Return `true` if the given Merkle root is known to the contract.
    pub fn is_known_root(env: Env, root: BytesN<32>) -> bool {
        pool::is_known_root(&env, &root)
    }

    // ── Admin ─────────────────────────────────────────────────────────

    /// Update the Groth16 verifying key.  Admin only.
    pub fn set_verifying_key_g16(
        env: Env,
        caller: Address,
        vk: Groth16VerifyingKey,
    ) -> Result<(), ZkError> {
        assert_admin(&env, &caller)?;
        caller.require_auth();
        env.storage().instance().set(&KEY_VK_G16, &vk);
        Ok(())
    }

    /// Update the PLONK verifying key.  Admin only.
    pub fn set_verifying_key_plonk(
        env: Env,
        caller: Address,
        vk: PlonkVerifyingKey,
    ) -> Result<(), ZkError> {
        assert_admin(&env, &caller)?;
        caller.require_auth();
        env.storage().instance().set(&KEY_VK_PLONK, &vk);
        Ok(())
    }

    /// Permissionless TTL extension for a list of nullifier hashes.
    ///
    /// Anyone can call this to prevent nullifier entries from expiring.
    pub fn extend_nullifier_ttl(env: Env, hashes: Vec<NullifierHash>) {
        nullifier::batch_extend_ttl(&env, &hashes);
    }

    // ── Private helpers ───────────────────────────────────────────────

    /// Load the Groth16 VK from Instance storage and call `groth16::verify_groth16`.
    fn do_verify_groth16(
        env: &Env,
        proof: &Groth16Proof,
        public_inputs: &PublicInputs,
        tracker: &mut BudgetTracker,
    ) -> Result<(), ZkError> {
        let vk: Groth16VerifyingKey = env
            .storage()
            .instance()
            .get(&KEY_VK_G16)
            .ok_or(ZkError::NotInitialised)?;

        // Pack shielded-pool inputs into the Vec<BytesN<32>> expected by
        // the Groth16 IC:  [merkle_root, nullifier_hash, recipient_hash, asset_id]
        let mut inputs: Vec<BytesN<32>> = Vec::new(env);
        inputs.push_back(public_inputs.merkle_root.clone());
        inputs.push_back(public_inputs.nullifier_hash.clone());
        inputs.push_back(public_inputs.recipient_hash.clone());
        inputs.push_back(public_inputs.asset_id.clone());

        groth16::verify_groth16(env, &vk, proof, &inputs, tracker)
    }

    /// Load the PLONK VK from Instance storage and call `plonk::verify_plonk`.
    fn do_verify_plonk(
        env: &Env,
        proof: &PlonkProof,
        public_inputs: &PublicInputs,
        tracker: &mut BudgetTracker,
    ) -> Result<(), ZkError> {
        let vk: PlonkVerifyingKey = env
            .storage()
            .instance()
            .get(&KEY_VK_PLONK)
            .ok_or(ZkError::NotInitialised)?;

        plonk::verify_plonk(env, &vk, proof, public_inputs, tracker)
    }
}
