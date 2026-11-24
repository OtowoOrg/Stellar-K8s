// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Stake delegation and reward tracking for validators that accept delegation.
//!
//! Delegated stake is refreshed from network data each epoch. Rewards are split
//! by fee share: the validator keeps a commission (basis points) and the rest is
//! distributed to delegators pro rata to stake. All amounts are integer stroops;
//! rounding remainders go to the validator so the ledger always sums exactly.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const BPS_DENOM: u128 = 10_000;

/// One reward-ledger entry for a delegator in an epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewardEntry {
    pub epoch: u64,
    pub delegator: String,
    pub stake: u64,
    pub reward: u64,
}

/// Result of distributing one epoch's reward.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochDistribution {
    pub epoch: u64,
    pub total_reward: u64,
    /// Commission plus rounding remainder kept by the validator.
    pub validator_share: u64,
    pub entries: Vec<RewardEntry>,
}

/// Tracks delegated stake and the historical reward ledger.
#[derive(Debug, Default)]
pub struct DelegationTracker {
    stake: BTreeMap<String, u64>,
    stake_epoch: u64,
    ledger: Vec<RewardEntry>,
}

impl DelegationTracker {
    /// Replace delegated stake with the snapshot read from the network.
    pub fn update_stake(&mut self, epoch: u64, snapshot: impl IntoIterator<Item = (String, u64)>) {
        self.stake = snapshot.into_iter().filter(|(_, s)| *s > 0).collect();
        self.stake_epoch = epoch;
    }

    pub fn total_delegated(&self) -> u64 {
        self.stake.values().sum()
    }

    /// Epoch of the last stake snapshot.
    pub fn stake_epoch(&self) -> u64 {
        self.stake_epoch
    }

    /// Distribute `total_reward` for `epoch` using the current stake snapshot
    /// and append the result to the ledger.
    pub fn distribute(
        &mut self,
        epoch: u64,
        total_reward: u64,
        commission_bps: u16,
    ) -> EpochDistribution {
        let commission_bps = u128::from(commission_bps).min(BPS_DENOM);
        let commission = (u128::from(total_reward) * commission_bps / BPS_DENOM) as u64;
        let pool = total_reward - commission;
        let total_stake = u128::from(self.total_delegated());

        let mut entries = Vec::new();
        let mut paid = 0u64;
        if total_stake > 0 {
            for (delegator, &stake) in &self.stake {
                let reward = (u128::from(pool) * u128::from(stake) / total_stake) as u64;
                paid += reward;
                entries.push(RewardEntry {
                    epoch,
                    delegator: delegator.clone(),
                    stake,
                    reward,
                });
            }
        }
        self.ledger.extend(entries.iter().cloned());
        EpochDistribution {
            epoch,
            total_reward,
            validator_share: total_reward - paid,
            entries,
        }
    }

    /// Ledger entries for a delegator (all epochs) in epoch order.
    pub fn history(&self, delegator: &str) -> Vec<&RewardEntry> {
        self.ledger
            .iter()
            .filter(|e| e.delegator == delegator)
            .collect()
    }

    /// Ledger entries for a single epoch.
    pub fn epoch_entries(&self, epoch: u64) -> Vec<&RewardEntry> {
        self.ledger.iter().filter(|e| e.epoch == epoch).collect()
    }

    /// Export a delegator statement as CSV (`epoch,stake,reward` plus a total row).
    pub fn statement_csv(&self, delegator: &str) -> String {
        let mut out = String::from("epoch,stake,reward\n");
        let mut total = 0u64;
        for e in self.history(delegator) {
            out.push_str(&format!("{},{},{}\n", e.epoch, e.stake, e.reward));
            total += e.reward;
        }
        out.push_str(&format!("total,,{total}\n"));
        out
    }

    /// Export a delegator statement as JSON.
    pub fn statement_json(&self, delegator: &str) -> serde_json::Result<String> {
        serde_json::to_string_pretty(&self.history(delegator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> DelegationTracker {
        let mut t = DelegationTracker::default();
        t.update_stake(
            7,
            vec![
                ("GA".to_string(), 600),
                ("GB".to_string(), 300),
                ("GC".to_string(), 100),
                ("GZ".to_string(), 0),
            ],
        );
        t
    }

    #[test]
    fn stake_snapshot_ignores_zero_and_sums() {
        let t = tracker();
        assert_eq!(t.total_delegated(), 1000);
        assert_eq!(t.stake_epoch(), 7);
    }

    #[test]
    fn distribution_matches_fee_share_rules() {
        let mut t = tracker();
        // 10% commission on 1000 -> pool 900, split 60/30/10.
        let d = t.distribute(7, 1000, 1000);
        let get = |a: &str| d.entries.iter().find(|e| e.delegator == a).unwrap().reward;
        assert_eq!(get("GA"), 540);
        assert_eq!(get("GB"), 270);
        assert_eq!(get("GC"), 90);
        assert_eq!(d.validator_share, 100);
    }

    #[test]
    fn rounding_remainder_goes_to_validator_and_total_is_exact() {
        let mut t = DelegationTracker::default();
        t.update_stake(1, vec![("A".into(), 1), ("B".into(), 1), ("C".into(), 1)]);
        let d = t.distribute(1, 100, 0);
        let paid: u64 = d.entries.iter().map(|e| e.reward).sum();
        assert_eq!(paid + d.validator_share, 100);
        assert_eq!(d.validator_share, 1);
    }

    #[test]
    fn no_delegators_keeps_everything() {
        let mut t = DelegationTracker::default();
        let d = t.distribute(1, 500, 500);
        assert!(d.entries.is_empty());
        assert_eq!(d.validator_share, 500);
    }

    #[test]
    fn history_and_statement_export() {
        let mut t = tracker();
        t.distribute(7, 1000, 1000);
        t.distribute(8, 2000, 1000);
        assert_eq!(t.history("GA").len(), 2);
        assert_eq!(t.epoch_entries(8).len(), 3);
        let csv = t.statement_csv("GA");
        assert_eq!(
            csv,
            "epoch,stake,reward\n7,600,540\n8,600,1080\ntotal,,1620\n"
        );
        assert!(t.statement_json("GA").unwrap().contains("\"reward\": 1080"));
    }
}
