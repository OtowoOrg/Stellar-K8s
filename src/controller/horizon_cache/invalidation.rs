//! Cache invalidation tied to ledger updates.

use serde::{Deserialize, Serialize};
use tracing::info;

use super::cache::HorizonCache;

/// Ledger update event triggering cache invalidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidationEvent {
    pub ledger_sequence: u64,
    pub closed_at: chrono::DateTime<chrono::Utc>,
    pub affected_prefixes: Vec<String>,
}

/// Invalidates cached queries when the ledger advances.
pub struct LedgerInvalidator;

impl LedgerInvalidator {
    /// Compute cache key prefixes affected by a ledger close.
    pub fn affected_prefixes(ledger_sequence: u64) -> Vec<String> {
        vec![
            format!("ledger:{ledger_sequence}:"),
            format!("ledgers:{ledger_sequence}:"),
            "transactions:recent:".to_string(),
            "payments:recent:".to_string(),
            "effects:recent:".to_string(),
        ]
    }

    /// Invalidate cache entries affected by a ledger close event.
    pub fn on_ledger_close(cache: &HorizonCache, ledger_sequence: u64) -> InvalidationEvent {
        let prefixes = Self::affected_prefixes(ledger_sequence);

        for prefix in &prefixes {
            cache.evict_by_prefix(prefix);
        }

        info!(
            ledger = ledger_sequence,
            prefixes = ?prefixes,
            "Cache invalidated for ledger close"
        );

        InvalidationEvent {
            ledger_sequence,
            closed_at: chrono::Utc::now(),
            affected_prefixes: prefixes,
        }
    }

    /// Invalidate a specific account's cached data.
    pub fn invalidate_account(cache: &HorizonCache, account_id: &str) {
        let prefix = format!("accounts:{account_id}");
        cache.evict_by_prefix(&prefix);
        info!(account = %account_id, "Account cache invalidated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::horizon_cache::HorizonCacheConfig;

    #[test]
    fn ledger_close_invalidates_entries() {
        let cache = HorizonCache::new(HorizonCacheConfig::default());
        cache.put("ledger:100:transactions", b"tx".to_vec());
        cache.put("ledger:200:transactions", b"tx2".to_vec());

        let event = LedgerInvalidator::on_ledger_close(&cache, 100);
        assert_eq!(event.ledger_sequence, 100);
        assert!(cache.get("ledger:100:transactions").is_none());
        assert!(cache.get("ledger:200:transactions").is_some());
    }
}
