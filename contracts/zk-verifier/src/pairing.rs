//! # pairing.rs
//!
//! BN254 bilinear pairing abstraction for the ZKP verifier.
//!
//! This module re-exports the canonical curve-point types from [`crate::types`]
//! and provides the pairing-check interface that will eventually delegate to a
//! Soroban host function.
//!
//! ## Architecture
//!
//! ```text
//!  groth16.rs  ──┐
//!                ├──► pairing.rs (pairing_check stub)
//!  plonk.rs    ──┘        │
//!                         ▼
//!              types.rs (G1Point, G2Point)
//! ```
//!
//! ## Host Function Roadmap
//!
//! | Protocol | Status          | Notes                                      |
//! |----------|-----------------|--------------------------------------------|
//! | ≤ 22     | Not available   | Structural checks only; stub returns true  |
//! | 23+      | Planned (CAP-?) | `env.crypto().bn254_pairing_check(&pairs)` |
//!
//! Once the host function is available, replace all uses of the internal
//! `pairing_check` stub in `groth16.rs` and `plonk.rs` with direct calls.

// Re-export the canonical types so callers can `use crate::pairing::*` as a
// convenience alias.
pub use crate::types::{G1Point, G2Point};
