//! Quorum-aware eviction-safety helpers for PDB auto-tuning.
//!
//! A validator that is still catching up must never be evicted
//! (`maxUnavailable = 0`). Even for synced validators, eviction is only safe
//! while enough quorum capacity remains.

/// Returns the `maxUnavailable` budget for a validator node.
///
/// * `syncing` — true while the node is still catching up with the network.
/// * `ready` — number of currently ready validators in the namespace.
/// * `quorum` — minimum number of validators required for SCP safety.
///
/// A syncing node, or a set that would drop to (or below) quorum, gets a
/// budget of `0`. Otherwise a single eviction is tolerated.
pub fn max_unavailable(syncing: bool, ready: usize, quorum: usize) -> i32 {
    if syncing || ready <= quorum {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncing_node_never_has_budget() {
        assert_eq!(max_unavailable(true, 5, 3), 0);
    }

    #[test]
    fn quorum_at_risk_has_no_budget() {
        assert_eq!(max_unavailable(false, 3, 3), 0);
        assert_eq!(max_unavailable(false, 2, 3), 0);
    }

    #[test]
    fn healthy_quorum_allows_single_eviction() {
        assert_eq!(max_unavailable(false, 4, 3), 1);
    }
}
