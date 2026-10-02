//! # errors.rs
//!
//! All contract-level error codes for the ZKP verifier.
//!
//! Error codes are stable across upgrades; never renumber or remove entries –
//! only append new ones.  Clients depend on the numeric values for structured
//! error handling.

use soroban_sdk::contracterror;

/// Errors emitted by the ZkVerifier and ShieldedPool contracts.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ZkError {
    // ── Proof / Input Errors (1–19) ─────────────────────────────────────
    /// The proof object is malformed (wrong lengths, invalid encoding).
    MalformedProof = 1,

    /// The verifying key stored in the contract does not match the expected
    /// circuit (wrong `ic` length vs number of public inputs).
    VerifyingKeyMismatch = 2,

    /// The number of public inputs provided does not match the verifying key.
    PublicInputLengthMismatch = 3,

    /// The elliptic-curve pairing check failed – the proof is invalid.
    PairingCheckFailed = 4,

    /// The PLONK polynomial evaluation check failed.
    PlonkEvalCheckFailed = 5,

    /// A field element is not a valid member of the scalar field Fr
    /// (i.e. it is >= the prime order).
    InvalidFieldElement = 6,

    /// A G1 or G2 point is not on the curve or is the point at infinity.
    InvalidCurvePoint = 7,

    // ── Nullifier / Replay-Protection Errors (20–39) ────────────────────
    /// The nullifier has already been spent; this is a replay attack.
    NullifierAlreadySpent = 20,

    /// The nullifier hash is the zero value, which is never valid.
    NullifierIsZero = 21,

    // ── Commitment Tree / Shielded Pool Errors (40–59) ──────────────────
    /// The Merkle root provided in the public inputs is not in the set of
    /// known historical roots stored by the contract.
    UnknownMerkleRoot = 40,

    /// The commitment tree is full (max depth / leaf count reached).
    CommitmentTreeFull = 41,

    /// The note commitment submitted is the zero value, which is never valid.
    InvalidNoteCommitment = 42,

    // ── Admin / Authorisation Errors (60–79) ────────────────────────────
    /// The caller is not the contract admin.
    Unauthorised = 60,

    /// The contract has been initialised already; `init` cannot be called
    /// again.
    AlreadyInitialised = 61,

    /// The contract has not been initialised yet.
    NotInitialised = 62,

    // ── Gas / Budget Errors (80–99) ──────────────────────────────────────
    /// The estimated CPU instruction count would exceed the per-transaction
    /// budget (`MAX_TX_CPU_INSTRUCTIONS`).  The proof was rejected before
    /// any expensive pairings were executed.
    CpuBudgetExceeded = 80,
}
