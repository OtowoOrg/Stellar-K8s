//! Adaptive query optimization for Horizon API requests.

use serde::{Deserialize, Serialize};

use super::cache::{CacheLayer, HorizonCache, HorizonCacheConfig};

/// Horizon query type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QueryType {
    Account,
    Payment,
    Transaction,
    OrderBook,
    Ledger,
    Effects,
    Operations,
}

/// Optimized query execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub cache_key: String,
    pub query_type: QueryType,
    pub cacheable: bool,
    pub preferred_layer: CacheLayer,
    pub ttl_secs: u64,
    pub use_compression: bool,
    pub prefetch_related: Vec<String>,
}

/// Adaptive query optimizer.
pub struct QueryOptimizer;

impl QueryOptimizer {
    /// Classify and optimize a Horizon query path.
    pub fn plan(path: &str, compression_enabled: bool) -> QueryPlan {
        let query_type = Self::classify(path);
        let cacheable = Self::is_cacheable(query_type, path);
        let preferred_layer = Self::preferred_layer(query_type);
        let ttl_secs = Self::ttl_for_type(query_type);
        let prefetch_related = Self::prefetch_keys(query_type, path);

        QueryPlan {
            cache_key: Self::cache_key(path),
            query_type,
            cacheable,
            preferred_layer,
            ttl_secs,
            use_compression: compression_enabled,
            prefetch_related,
        }
    }

    /// Execute a query with cache-aside pattern.
    pub fn execute<F>(cache: &HorizonCache, plan: &QueryPlan, fetch: F) -> Vec<u8>
    where
        F: FnOnce() -> Vec<u8>,
    {
        if plan.cacheable {
            if let Some((data, _)) = cache.get(&plan.cache_key) {
                return data;
            }
        }

        let data = fetch();

        if plan.cacheable {
            cache.put(&plan.cache_key, data.clone());
        }

        data
    }

    fn classify(path: &str) -> QueryType {
        if path.contains("/accounts/") {
            QueryType::Account
        } else if path.contains("/payments") {
            QueryType::Payment
        } else if path.contains("/transactions") {
            QueryType::Transaction
        } else if path.contains("/order_book") {
            QueryType::OrderBook
        } else if path.contains("/ledgers/") {
            QueryType::Ledger
        } else if path.contains("/effects") {
            QueryType::Effects
        } else {
            QueryType::Operations
        }
    }

    fn is_cacheable(query_type: QueryType, path: &str) -> bool {
        // Order book and recent ledger data are less cacheable
        match query_type {
            QueryType::OrderBook => false,
            QueryType::Ledger if path.contains("cursor") => false,
            _ => true,
        }
    }

    fn preferred_layer(query_type: QueryType) -> CacheLayer {
        match query_type {
            QueryType::Account | QueryType::Payment => CacheLayer::L1Memory,
            QueryType::Transaction | QueryType::Operations => CacheLayer::L2Redis,
            QueryType::Ledger | QueryType::Effects => CacheLayer::L3Cdn,
            QueryType::OrderBook => CacheLayer::L1Memory,
        }
    }

    fn ttl_for_type(query_type: QueryType) -> u64 {
        match query_type {
            QueryType::Account => 300,
            QueryType::Payment => 120,
            QueryType::Transaction => 60,
            QueryType::OrderBook => 5,
            QueryType::Ledger => 3600,
            QueryType::Effects => 300,
            QueryType::Operations => 120,
        }
    }

    fn prefetch_keys(query_type: QueryType, path: &str) -> Vec<String> {
        match query_type {
            QueryType::Account => {
                vec![
                    format!("{path}/payments"),
                    format!("{path}/transactions"),
                    format!("{path}/effects"),
                ]
            }
            QueryType::Ledger => {
                vec![format!("{path}/transactions"), format!("{path}/operations")]
            }
            _ => Vec::new(),
        }
    }

    fn cache_key(path: &str) -> String {
        path.trim_start_matches('/').replace('/', ":")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_account_query() {
        let plan = QueryOptimizer::plan("/accounts/GABC123", true);
        assert_eq!(plan.query_type, QueryType::Account);
        assert!(plan.cacheable);
    }

    #[test]
    fn order_book_not_cacheable() {
        let plan = QueryOptimizer::plan("/order_book?selling=X&buying=Y", true);
        assert!(!plan.cacheable);
    }

    #[test]
    fn execute_uses_cache() {
        let cache = HorizonCache::new(HorizonCacheConfig {
            l2_redis_enabled: false,
            l3_cdn_enabled: false,
            ..Default::default()
        });
        let plan = QueryOptimizer::plan("/accounts/GABC", true);

        let mut call_count = 0;
        let fetch = || {
            call_count += 1;
            b"fetched".to_vec()
        };

        QueryOptimizer::execute(&cache, &plan, fetch);
        QueryOptimizer::execute(&cache, &plan, fetch);
        assert_eq!(call_count, 1);
    }
}
