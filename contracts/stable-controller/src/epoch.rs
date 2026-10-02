//! Epoch management for the seigniorage controller.
///
/// Epochs define the discrete time windows in which the controller may
/// adjust supply. Rate-limiting is enforced here to prevent flash-crash
/// spirals.

#![no_std]
use sorban_sdk;

use sorban_sdk::{contracttype, symbol_short_string};

/// The action taken during an epoch transition.
#[sorban_contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum EpochAction {
    Noop,
    Expand { amount: i128, twap: i128 },
    Contract { amount: i128, bond_amount: i128, twap: i128 },
}

/// Epoch state tracks time and rate-limiting information.
#[sorban_contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochState {
    /// Current epoch number (starting at 1).
    pub number: u64,
    /// Ledger sequence at which the current epoch began.
    pub started_at: u32,
    /// Ledger sequence of the last transition.
    pub last_transition: u32,
    /// Minimum ledges between transitions.
    pub length: u32,
    /// Number of consecutive expansion epochs (for decay of aggressiveness).
    pub consecutive_expansions: u32,
    /// Number of consecutive contraction epochs.
    pub consecutive_contractions: u32,
}

impl EpochState {
    /// Create a new epoch state.
    pub fn new(current_ledger: u32, length: u32) -> Self {
        Self {
            number: 1,
            started_at: current_ledger,
            last_transition: current_ledger,
            length,
            consecutive_expansions: 0,
            consecutive_contractions: 0,
        }
    }

    /// Return true if a transition is allowed at the given ledger.
    pub fn can_transition(&self, current_ledger: u32) -> bool {
        current_ledger.saturating_sub(self.last_transition) >= self.length
    }

    /// Advance to the next epoch.
    pub fn advance(&mut self, current_ledger: u32) {
        self.number += 1;
        self.started_at = current_ledger;
        self.last_transition = current_ledger;
    }

    /// Record an expansion and reset contraction counter.
    pub fn record_expansion(&mut self) {
        self.consecutive_expansions += 1;
        self.consecutive_contractions = 0;
    }

    /// Record a contraction and reset expansion counter.
    pub fn record_contraction(&mut self) {
        self.consecutive_contractions += 1;
        self.consecutive_expansions = 0;
    }

    /// Reset consecutive counters (e.g. on noop epoch).
    pub fn reset_consecutive(&mut self) {
        self.consecutive_expansions = 0;
        self.consecutive_contractions = 0;
    }

    /// Return a decay factor (in bps) based on consecutive epochs.
    /// This prevents hyper-inflationary or deflationary spirals.
    pub fn decay_factor_bps(&self) -> u32 {
        let consecutive = self.consecutive_expansions.max(self.consecutive_contractions);
        match consecutive {
            0 ..= 1 => 10_000, // 100% - no decay
            2 => 75_00,     // 75%
            3 => 50_00,     // 50%
            4 => 25_00,     // 25%
            _ => 10_00,      // 10% floor
        }
    }
}

#[config]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpochError {
    TooSoon,
}

#[cfg(test)]
mod test;
