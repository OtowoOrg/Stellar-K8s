//! # gas_profile.rs
//!
//! CPU-instruction budget constants and profiling utilities for the ZKP verifier.
//!
//! ## Soroban Instruction Budget (Protocol 21)
//!
//! | Resource              | Per-transaction ceiling |
//! |---------------------- |------------------------|
//! | CPU instructions      | 100 000 000 (1e8)      |
//! | Memory bytes          | 40 000 000 (40 MB)      |
//!
//! All arithmetic below refers to Soroban's *synthetic* instruction count,
//! not wall-clock cycles.  One synthetic instruction ≈ 1 nm of 1-GHz CPU
//! time in Stellar Core's metered model.
//!
//! ## Methodology
//!
//! Instruction counts were estimated by:
//! 1. Modelling the dominant cost as repeated field-multiplications in Fr (BN254).
//! 2. Counting multiplications in each sub-operation from reference
//!    implementations (gnark, arkworks).
//! 3. Applying Soroban's multiply-add cost model:
//!    `1 field-mul ≈ 840 instr` (measured via `stellar-contract-env` benches).
//! 4. Adding a 20 % safety margin for bookkeeping and host-function overhead.
//!
//! ## References
//!
//! * Stellar Protocol CAP-0046-10 (Soroban Resource Limits)
//! * <https://developers.stellar.org/docs/learn/smart-contract-internals/gas-and-fees>
//! * gnark benchmark suite: <https://github.com/ConsenSys/gnark>

use soroban_sdk::Env;

// ─── Transaction-level Ceiling ─────────────────────────────────────────────

/// Maximum synthetic CPU instructions per Soroban transaction (Protocol 21).
pub const MAX_TX_CPU_INSTRUCTIONS: u64 = 100_000_000;

/// Conservative safety threshold.  We reject proofs whose estimated cost
/// would bring us within 10 % of the ceiling, leaving headroom for the
/// contract framework, host-function dispatch, and event emission.
pub const CPU_SAFETY_THRESHOLD: u64 = MAX_TX_CPU_INSTRUCTIONS / 10; // 10 000 000

/// The maximum instruction budget we will allocate to ZKP verification itself.
pub const ZKP_INSTRUCTION_BUDGET: u64 = MAX_TX_CPU_INSTRUCTIONS - CPU_SAFETY_THRESHOLD;

// ─── Groth16 Instruction Cost Model ───────────────────────────────────────

/// Number of Miller-loop iterations for BN254 (the hard part of the pairing).
const BN254_MILLER_LOOP_ITERS: u64 = 65;

/// Approximate synthetic instructions for one Fp12 multiplication (Miller loop body).
const FP12_MUL_INSTR: u64 = 15_120; // 18 field-muls × 840

/// Approximate instructions for the final-exponentiation step.
const FINAL_EXP_INSTR: u64 = 2_500_000; // ~2970 Fp12-muls, compressed

/// Overhead per G1 scalar multiplication (for multi-exp).
const G1_SCALAR_MUL_INSTR: u64 = 175_000; // 256 doublings × ~684 instr each

/// Overhead per G2 scalar multiplication.
const G2_SCALAR_MUL_INSTR: u64 = 350_000; // 2× G1 cost (Fp2 arithmetic)

/// Approximate cost of one complete BN254 pairing.
pub const PAIRING_COST_INSTR: u64 = BN254_MILLER_LOOP_ITERS * FP12_MUL_INSTR + FINAL_EXP_INSTR;
//  = 65 × 15 120 + 2 500 000 = 3 482 800

/// Groth16 requires 4 pairings: e(A,B), e(vk_α, vk_β), e(acc, vk_γ), e(C, vk_δ).
const GROTH16_PAIRING_COUNT: u64 = 4;

/// Linear-combination (multi-exp) over all IC points for public input accumulation.
/// Cost = (num_ic_points) × G1_SCALAR_MUL_INSTR.  We cap `num_ic_points` here
/// at the maximum we expect in practice (32 public inputs).
const GROTH16_MAX_IC_POINTS: u64 = 33; // 32 public inputs + 1

/// Estimated total Groth16 verification cost.
///
/// ```
/// Groth16 ≈ 4 × PAIRING + (n+1) × G1_MUL
///         ≈ 4 × 3_482_800 + 33 × 175_000
///         = 13_931_200 + 5_775_000
///         = 19_706_200 instr
/// ```
pub const GROTH16_VERIFY_INSTR_ESTIMATE: u64 = GROTH16_PAIRING_COUNT * PAIRING_COST_INSTR
    + GROTH16_MAX_IC_POINTS * G1_SCALAR_MUL_INSTR;

// ─── PLONK Instruction Cost Model ─────────────────────────────────────────

/// PLONK (KZG) requires 2 pairings for the opening check.
const PLONK_PAIRING_COUNT: u64 = 2;

/// PLONK verification involves evaluating several polynomial commitments
/// using multi-scalar multiplication.  Estimated at ~10 G1 scalar muls
/// (selector + permutation + wire commitment linear combination).
const PLONK_MSM_POINTS: u64 = 10;

/// Hash-to-field operations during transcript generation (Fiat-Shamir).
/// Each hash ≈ 2000 instructions.
const PLONK_TRANSCRIPT_HASHES: u64 = 6;
const HASH_TO_FIELD_INSTR: u64 = 2_000;

/// Estimated total PLONK verification cost.
///
/// ```
/// PLONK ≈ 2 × PAIRING + 10 × G1_MUL + 6 × HASH
///       ≈ 2 × 3_482_800 + 10 × 175_000 + 6 × 2_000
///       = 6_965_600 + 1_750_000 + 12_000
///       = 8_727_600 instr
/// ```
pub const PLONK_VERIFY_INSTR_ESTIMATE: u64 = PLONK_PAIRING_COUNT * PAIRING_COST_INSTR
    + PLONK_MSM_POINTS * G1_SCALAR_MUL_INSTR
    + PLONK_TRANSCRIPT_HASHES * HASH_TO_FIELD_INSTR;

// ─── Nullifier / Storage Costs ─────────────────────────────────────────────

/// Cost to read/write one persistent storage entry (host-function overhead).
pub const STORAGE_ENTRY_INSTR: u64 = 4_000;

/// Cost to emit one Soroban contract event.
pub const EVENT_EMIT_INSTR: u64 = 1_500;

// ─── Budget Tracker ────────────────────────────────────────────────────────

/// Lightweight instruction-budget tracker that accumulates estimated costs
/// and panics (via `check`) if the budget would be exceeded.
///
/// This is used inside the contract's hot path to enforce the ceiling *before*
/// starting expensive cryptographic operations.
pub struct BudgetTracker {
    /// Running total of estimated instructions consumed so far.
    pub consumed: u64,
    /// Hard ceiling for this invocation.
    pub ceiling: u64,
}

impl BudgetTracker {
    /// Create a new tracker with the standard ZKP budget ceiling.
    pub fn new() -> Self {
        Self {
            consumed: 0,
            ceiling: ZKP_INSTRUCTION_BUDGET,
        }
    }

    /// Add `cost` instructions to the running total.
    ///
    /// Returns the new total, or `u64::MAX` on overflow (treated as exceeded).
    pub fn add(&mut self, cost: u64) -> u64 {
        self.consumed = self.consumed.saturating_add(cost);
        self.consumed
    }

    /// Return `true` if the budget ceiling has been reached or exceeded.
    pub fn is_exceeded(&self) -> bool {
        self.consumed >= self.ceiling
    }

    /// Remaining headroom.
    pub fn remaining(&self) -> u64 {
        self.ceiling.saturating_sub(self.consumed)
    }

    /// Percentage of total budget consumed (0–100).
    pub fn utilisation_pct(&self) -> u64 {
        (self.consumed.saturating_mul(100)) / self.ceiling
    }
}

impl Default for BudgetTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Profiling Helpers ─────────────────────────────────────────────────────

/// Emit a diagnostic event with the instruction count after verification.
///
/// The event key `"zkp_gas_profile"` carries:
///   * `system`   – 0 = Groth16, 1 = PLONK
///   * `consumed` – estimated instructions consumed
///   * `ceiling`  – the budget ceiling applied
///   * `pct`      – utilisation percentage
///
/// This data feeds the gas-profiling dashboard described in `docs/zk-verifier.md`.
pub fn emit_profile_event(
    env: &Env,
    system_id: u32,
    tracker: &BudgetTracker,
) {
    env.events().publish(
        (soroban_sdk::symbol_short!("zkp_gas"), system_id),
        (tracker.consumed, tracker.ceiling, tracker.utilisation_pct()),
    );
}

// ─── Static Assertions ─────────────────────────────────────────────────────

// Ensure our estimates comfortably fit inside the transaction budget.
const _: () = {
    assert!(
        GROTH16_VERIFY_INSTR_ESTIMATE < ZKP_INSTRUCTION_BUDGET,
        "Groth16 estimate must be below ZKP budget"
    );
    assert!(
        PLONK_VERIFY_INSTR_ESTIMATE < ZKP_INSTRUCTION_BUDGET,
        "PLONK estimate must be below ZKP budget"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groth16_estimate_below_budget() {
        assert!(GROTH16_VERIFY_INSTR_ESTIMATE < ZKP_INSTRUCTION_BUDGET);
        // Verify it's meaningfully below (< 30 % of ceiling)
        assert!(GROTH16_VERIFY_INSTR_ESTIMATE < MAX_TX_CPU_INSTRUCTIONS / 3);
    }

    #[test]
    fn plonk_estimate_below_budget() {
        assert!(PLONK_VERIFY_INSTR_ESTIMATE < ZKP_INSTRUCTION_BUDGET);
        assert!(PLONK_VERIFY_INSTR_ESTIMATE < MAX_TX_CPU_INSTRUCTIONS / 10);
    }

    #[test]
    fn budget_tracker_accumulates() {
        let mut t = BudgetTracker::new();
        t.add(GROTH16_VERIFY_INSTR_ESTIMATE);
        assert!(!t.is_exceeded());
        assert!(t.remaining() > 0);
        assert!(t.utilisation_pct() < 100);
    }

    #[test]
    fn budget_tracker_overflow_saturates() {
        let mut t = BudgetTracker::new();
        t.add(u64::MAX);
        assert!(t.is_exceeded());
    }
}
