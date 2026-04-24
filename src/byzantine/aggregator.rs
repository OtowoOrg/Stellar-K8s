//! Aggregator — collects observations from multiple watchers and computes
//! the divergence ratio used to detect Byzantine partitions.
//!
//! The aggregator is used by the central Prometheus evaluation rule, but this
//! Rust module provides the same logic for:
//! - Unit testing the divergence calculation.
//! - An optional in-process aggregation endpoint (useful when Prometheus
//!   federation is not available).

use std::collections::HashMap;

use chrono::Utc;

use super::types::{AggregatedConsensusView, ConsensusObservation};

/// Threshold above which a Byzantine partition alert fires.
pub const BYZANTINE_DIVERGENCE_THRESHOLD: f64 = 0.20;

/// Aggregate a slice of observations (one per watcher) into a consensus view.
///
/// # Algorithm
///
/// 1. Group observations by `network`.
/// 2. For each network, count how many watchers report each unique ledger hash.
/// 3. The *majority hash* is the hash reported by the most watchers.
/// 4. Watchers that report a different hash are *diverging*.
/// 5. `divergence_ratio = diverging_watchers / total_watchers`.
/// 6. If `divergence_ratio > 0.20`, set `byzantine_alert = true`.
///
/// Observations older than `max_age_secs` are excluded (stale watcher).
pub fn aggregate(
    observations: &[ConsensusObservation],
    max_age_secs: i64,
) -> Vec<AggregatedConsensusView> {
    let now = Utc::now();

    // Filter stale observations.
    let fresh: Vec<&ConsensusObservation> = observations
        .iter()
        .filter(|o| (now - o.observed_at).num_seconds() <= max_age_secs)
        .collect();

    // Group by network.
    let mut by_network: HashMap<&str, Vec<&ConsensusObservation>> = HashMap::new();
    for obs in &fresh {
        by_network.entry(obs.network.as_str()).or_default().push(obs);
    }

    let mut results = Vec::new();

    for (network, obs_list) in by_network {
        // Count votes per hash.
        let mut hash_votes: HashMap<&str, (usize, u64)> = HashMap::new(); // hash → (count, max_seq)
        for obs in &obs_list {
            let entry = hash_votes.entry(obs.ledger_hash.as_str()).or_insert((0, 0));
            entry.0 += 1;
            if obs.ledger_sequence > entry.1 {
                entry.1 = obs.ledger_sequence;
            }
        }

        // Find majority hash (most votes; tie-break by highest sequence).
        let majority = hash_votes
            .iter()
            .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then(a.1 .1.cmp(&b.1 .1)));

        let (majority_hash, majority_count, majority_seq) = match majority {
            Some((h, (count, seq))) => (*h, *count, *seq),
            None => continue,
        };

        let total = obs_list.len();
        let diverging: Vec<String> = obs_list
            .iter()
            .filter(|o| o.ledger_hash != majority_hash)
            .map(|o| o.watcher_id.clone())
            .collect();

        let diverging_count = diverging.len();
        let divergence_ratio = diverging_count as f64 / total as f64;
        let byzantine_alert = divergence_ratio > BYZANTINE_DIVERGENCE_THRESHOLD;

        if byzantine_alert {
            tracing::warn!(
                network,
                divergence_ratio,
                diverging_watchers = diverging_count,
                total_watchers = total,
                majority_hash,
                "⚠️  Byzantine partition detected: divergence ratio {:.1}% exceeds threshold {:.0}%",
                divergence_ratio * 100.0,
                BYZANTINE_DIVERGENCE_THRESHOLD * 100.0,
            );
        }

        results.push(AggregatedConsensusView {
            network: network.to_string(),
            total_watchers: total,
            agreeing_watchers: majority_count,
            diverging_watchers: diverging_count,
            majority_hash: majority_hash.to_string(),
            majority_sequence: majority_seq,
            divergence_ratio,
            byzantine_alert,
            diverging_watcher_ids: diverging,
            aggregated_at: now,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_obs(watcher_id: &str, region: &str, hash: &str, seq: u64) -> ConsensusObservation {
        ConsensusObservation {
            watcher_id: watcher_id.to_string(),
            cloud: "aws".to_string(),
            region: region.to_string(),
            network: "testnet".to_string(),
            node_endpoint: "http://localhost:11626".to_string(),
            ledger_sequence: seq,
            ledger_hash: hash.to_string(),
            is_externalized: true,
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn test_no_divergence_all_agree() {
        let obs = vec![
            make_obs("w1", "us-east-1", "aabbcc", 1000),
            make_obs("w2", "eu-west-1", "aabbcc", 1000),
            make_obs("w3", "ap-south-1", "aabbcc", 1000),
            make_obs("w4", "us-west-2", "aabbcc", 1000),
            make_obs("w5", "sa-east-1", "aabbcc", 1000),
        ];
        let views = aggregate(&obs, 300);
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.divergence_ratio, 0.0);
        assert!(!view.byzantine_alert);
        assert_eq!(view.diverging_watchers, 0);
    }

    #[test]
    fn test_exactly_20_percent_divergence_no_alert() {
        // 1 out of 5 = 20% — threshold is *strictly greater than* 20%.
        let obs = vec![
            make_obs("w1", "us-east-1", "aabbcc", 1000),
            make_obs("w2", "eu-west-1", "aabbcc", 1000),
            make_obs("w3", "ap-south-1", "aabbcc", 1000),
            make_obs("w4", "us-west-2", "aabbcc", 1000),
            make_obs("w5", "sa-east-1", "deadbeef", 999), // diverging
        ];
        let views = aggregate(&obs, 300);
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert!((view.divergence_ratio - 0.20).abs() < 1e-9);
        // 0.20 is NOT > 0.20, so no alert.
        assert!(!view.byzantine_alert);
    }

    #[test]
    fn test_above_20_percent_triggers_alert() {
        // 2 out of 5 = 40% — should alert.
        let obs = vec![
            make_obs("w1", "us-east-1", "aabbcc", 1000),
            make_obs("w2", "eu-west-1", "aabbcc", 1000),
            make_obs("w3", "ap-south-1", "aabbcc", 1000),
            make_obs("w4", "us-west-2", "deadbeef", 999),
            make_obs("w5", "sa-east-1", "deadbeef", 999),
        ];
        let views = aggregate(&obs, 300);
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert!((view.divergence_ratio - 0.40).abs() < 1e-9);
        assert!(view.byzantine_alert);
        assert_eq!(view.diverging_watcher_ids.len(), 2);
    }

    #[test]
    fn test_stale_observations_excluded() {
        use chrono::Duration;
        let mut old_obs = make_obs("w_stale", "us-east-1", "deadbeef", 500);
        // Make it 10 minutes old.
        old_obs.observed_at = Utc::now() - Duration::seconds(600);

        let obs = vec![
            make_obs("w1", "us-east-1", "aabbcc", 1000),
            make_obs("w2", "eu-west-1", "aabbcc", 1000),
            make_obs("w3", "ap-south-1", "aabbcc", 1000),
            old_obs,
        ];
        // max_age = 300s — the stale observation should be excluded.
        let views = aggregate(&obs, 300);
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.total_watchers, 3);
        assert_eq!(view.divergence_ratio, 0.0);
    }

    #[test]
    fn test_majority_hash_is_most_common() {
        // 3 watchers on hash A, 2 on hash B — majority is A.
        let obs = vec![
            make_obs("w1", "us-east-1", "hashA", 1000),
            make_obs("w2", "eu-west-1", "hashA", 1000),
            make_obs("w3", "ap-south-1", "hashA", 1000),
            make_obs("w4", "us-west-2", "hashB", 999),
            make_obs("w5", "sa-east-1", "hashB", 999),
        ];
        let views = aggregate(&obs, 300);
        let view = &views[0];
        assert_eq!(view.majority_hash, "hashA");
        assert_eq!(view.agreeing_watchers, 3);
        assert_eq!(view.diverging_watchers, 2);
        assert!((view.divergence_ratio - 0.40).abs() < 1e-9);
        assert!(view.byzantine_alert);
    }

    #[test]
    fn test_multiple_networks_aggregated_independently() {
        let mut mainnet_obs = make_obs("w1", "us-east-1", "mainnet_hash", 2000);
        mainnet_obs.network = "mainnet".to_string();
        let mut testnet_obs = make_obs("w2", "eu-west-1", "testnet_hash", 1000);
        testnet_obs.network = "testnet".to_string();

        let obs = vec![mainnet_obs, testnet_obs];
        let views = aggregate(&obs, 300);
        assert_eq!(views.len(), 2);
    }
}
