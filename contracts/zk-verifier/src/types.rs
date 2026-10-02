//! # types.rs
//!
//! Shared Soroban contract types for the ZKP verifier.
//!
//! Covers both Groth16 and PLONK proof systems, public inputs, the shielded
//! pool note commitment, and nullifier hashes.  All types derive
//! `contracttype` so they can be stored in ledger entries and passed through
//! the Soroban host ABI without additional serialisation.

#![allow(clippy::module_name_repetitions)]

use soroban_sdk::{contracttype, BytesN, Vec};

// ─── Elliptic-Curve Group Elements ─────────────────────────────────────────

/// An affine point on the G1 group of BLS12-381 or BN254.
///
/// Each coordinate is serialised as a 32-byte big-endian field element.
/// For BLS12-381 the actual field size is 48 bytes; the two "halves"
/// (high/low) are stored in `x` and `y` respectively to stay within the
/// 32-byte `BytesN` limit enforced by Soroban.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct G1Point {
    /// x-coordinate (big-endian, 32 bytes)
    pub x: BytesN<32>,
    /// y-coordinate (big-endian, 32 bytes)
    pub y: BytesN<32>,
}

/// An affine point on the G2 group (degree-2 extension field).
///
/// Each coordinate component is a pair of 32-byte field elements that
/// together represent one Fp2 element: `x = (x0, x1)` where the field
/// element is `x0 + x1·i`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct G2Point {
    /// x-coordinate in Fp2: (c0, c1)
    pub x: (BytesN<32>, BytesN<32>),
    /// y-coordinate in Fp2: (c0, c1)
    pub y: (BytesN<32>, BytesN<32>),
}

// ─── Groth16 Types ─────────────────────────────────────────────────────────

/// Groth16 verifying key (circuit-specific, generated offline by the
/// trusted-setup ceremony).
///
/// Stored in contract Instance storage so callers do not need to supply it on
/// every invocation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Groth16VerifyingKey {
    /// α·G1 – mixed with the proof A element in the pairing check.
    pub alpha_g1: G1Point,
    /// β·G2 – mixed with the proof B element.
    pub beta_g2: G2Point,
    /// γ·G2 – used to verify the public-input accumulator.
    pub gamma_g2: G2Point,
    /// δ·G2 – used to verify the hidden-input part of the proof.
    pub delta_g2: G2Point,
    /// [γ⁻¹·(β·u_i(x) + α·v_i(x) + w_i(x))]·G1 for each public input *i*.
    /// Length must equal `num_public_inputs + 1`.
    pub ic: Vec<G1Point>,
}

/// A Groth16 proof π = (A, B, C).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Groth16Proof {
    /// π_A – G1 element
    pub a: G1Point,
    /// π_B – G2 element
    pub b: G2Point,
    /// π_C – G1 element
    pub c: G1Point,
}

// ─── PLONK Types ───────────────────────────────────────────────────────────

/// PLONK/UltraPLONK proof (Kate-commitment based).
///
/// Field elements are 32-byte scalars in the BN254 or BLS12-381 scalar field.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlonkProof {
    /// Wire polynomial commitments [a], [b], [c]
    pub wire_commitments: Vec<G1Point>,
    /// Grand-product / permutation commitment [z]
    pub z_commitment: G1Point,
    /// Split quotient commitments [t_lo], [t_mid], [t_hi]
    pub t_commitments: Vec<G1Point>,
    /// Linearisation polynomial commitment [r]
    pub r_commitment: G1Point,
    /// Opening commitment [W_ζ]
    pub w_zeta: G1Point,
    /// Opening commitment [W_{ζω}]
    pub w_zeta_omega: G1Point,
    /// Evaluation of wire polynomial a(ζ)
    pub a_eval: BytesN<32>,
    /// Evaluation of wire polynomial b(ζ)
    pub b_eval: BytesN<32>,
    /// Evaluation of wire polynomial c(ζ)
    pub c_eval: BytesN<32>,
    /// Evaluation of the first permutation polynomial σ_1(ζ)
    pub sigma1_eval: BytesN<32>,
    /// Evaluation of the second permutation polynomial σ_2(ζ)
    pub sigma2_eval: BytesN<32>,
    /// Evaluation of the grand-product polynomial z(ζω)
    pub z_omega_eval: BytesN<32>,
}

/// PLONK verification key (circuit-specific, derived from circuit description).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlonkVerifyingKey {
    /// Commitment to the selector polynomial q_M
    pub q_m: G1Point,
    /// Commitment to the selector polynomial q_L
    pub q_l: G1Point,
    /// Commitment to the selector polynomial q_R
    pub q_r: G1Point,
    /// Commitment to the selector polynomial q_O
    pub q_o: G1Point,
    /// Commitment to the selector polynomial q_C
    pub q_c: G1Point,
    /// Commitment to the first permutation polynomial σ_1
    pub sigma1: G1Point,
    /// Commitment to the second permutation polynomial σ_2
    pub sigma2: G1Point,
    /// Commitment to the third permutation polynomial σ_3
    pub sigma3: G1Point,
    /// x·G2 – the SRS point for verification
    pub x2: G2Point,
    /// Number of public inputs the circuit exposes
    pub num_public_inputs: u32,
}

// ─── Public Inputs ─────────────────────────────────────────────────────────

/// Packed public inputs for a private-transfer proof.
///
/// The verifier checks these against the proof without revealing the private
/// amounts or addresses that were committed to inside the circuit.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicInputs {
    /// Merkle root of the shielded note commitment tree
    pub merkle_root: BytesN<32>,
    /// Nullifier hash – must not already exist in the nullifier registry
    pub nullifier_hash: BytesN<32>,
    /// Hash of the recipient's stealth address (kept private inside circuit)
    pub recipient_hash: BytesN<32>,
    /// Public "asset identifier" (e.g. Stellar asset contract address hash)
    pub asset_id: BytesN<32>,
    /// Fee paid to the relayer, in stroops (public so relayer can verify)
    pub relayer_fee: u64,
}

// ─── Shielded Pool Types ────────────────────────────────────────────────────

/// A shielded note commitment stored in the on-chain commitment tree.
///
/// The actual note (recipient, amount, asset, blinding factor) is kept
/// entirely off-chain; only the Pedersen commitment is stored on-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteCommitment {
    /// 32-byte Pedersen commitment to the note
    pub commitment: BytesN<32>,
    /// Ledger sequence number at which this commitment was inserted
    pub inserted_at: u32,
    /// Incremental index in the Merkle commitment tree
    pub leaf_index: u32,
}

/// A nullifier hash uniquely identifies a spent note.
///
/// The nullifier is computed inside the ZK circuit as:
///   `nullifier = PRF(note_secret_key, leaf_index)`
///
/// It is stored in Persistent storage so replay attacks are prevented
/// across ledger boundaries and contract upgrades.
pub type NullifierHash = BytesN<32>;

/// Supported proof systems.  The shielded pool accepts both so circuit
/// developers can choose the system that best fits their latency / proof-size
/// tradeoffs.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProofSystem {
    Groth16 = 0,
    Plonk = 1,
}

/// Union of both proof variants accepted by the shielded pool.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnyProof {
    Groth16(Groth16Proof),
    Plonk(PlonkProof),
}

/// Result of a successful proof verification returned to the caller.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyResult {
    /// The nullifier that was spent (caller should relay this to the pool)
    pub nullifier_hash: NullifierHash,
    /// Merkle root that was used for the membership proof
    pub merkle_root: BytesN<32>,
    /// Which proof system was used
    pub proof_system: ProofSystem,
    /// Gas / CPU instruction cost measured for this proof (informational)
    pub cpu_instructions: u64,
}
