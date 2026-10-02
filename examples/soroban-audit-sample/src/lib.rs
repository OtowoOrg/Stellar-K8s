//! # DELIBERATELY VULNERABLE — DO NOT DEPLOY
//!
//! Two small Soroban contracts with injected vulnerabilities, used to validate
//! `docs/templates/audit-checklist.md`. See `AUDIT.md` for the applied checklist
//! and `src/test.rs` for executable proofs of each finding.
//!
//! - [`vault::Vault`] holds token deposits and mirrors each user's balance into
//!   the lending contract as collateral.
//! - [`lending::Lending`] lends the same token against that mirrored collateral
//!   at a 50% loan-to-value ratio.
#![no_std]

pub mod lending;
pub mod vault;

#[cfg(test)]
mod test;
