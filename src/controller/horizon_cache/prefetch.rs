//! Predictive prefetching engine for Horizon queries.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::cache::HorizonCache;
use super::optimizer::{QueryOptimizer, QueryType};

/// Prefetch prediction based on access patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefetchPrediction {
    pub key: String,
    pub confidence: f64,
    pub query_type: QueryType,
}

/// Predictive prefetching engine using access frequency analysis.
pub struct PrefetchEngine {
    access_counts: HashMap<String, u64>,
    co_occurrence: HashMap<String, HashMap<String, u64>>,
    last_accessed: Option<String>,
}

impl PrefetchEngine {
    pub fn new() -> Self {
        Self {
            access_counts: HashMap::new(),
            co_occurrence: HashMap::new(),
            last_accessed: None,
        }
    }

    /// Record a query access for pattern learning.
    pub fn record_access(&mut self, path: &str) {
        let key = QueryOptimizer::plan(path, false).cache_key;
        *self.access_counts.entry(key.clone()).or_insert(0) += 1;

        if let Some(prev) = &self.last_accessed {
            self.co_occurrence
                .entry(prev.clone())
                .or_default()
                .entry(key.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
        }
        self.last_accessed = Some(key);
    }

    /// Predict keys to prefetch based on current access.
    pub fn predict(&self, path: &str, top_n: usize) -> Vec<PrefetchPrediction> {
        let key = QueryOptimizer::plan(path, false).cache_key;
        let plan = QueryOptimizer::plan(path, false);

        let mut predictions: Vec<PrefetchPrediction> = plan
            .prefetch_related
            .iter()
            .map(|related| PrefetchPrediction {
                key: related.clone(),
                confidence: 0.7,
                query_type: plan.query_type,
            })
            .collect();

        if let Some(co) = self.co_occurrence.get(&key) {
            let total: u64 = co.values().sum();
            for (related_key, count) in co {
                let confidence = *count as f64 / total as f64;
                if confidence > 0.1 {
                    predictions.push(PrefetchPrediction {
                        key: related_key.clone(),
                        confidence,
                        query_type: plan.query_type,
                    });
                }
            }
        }

        predictions.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        predictions.truncate(top_n);
        predictions
    }

    /// Prefetch predicted queries into cache.
    pub fn prefetch<F>(&self, cache: &HorizonCache, path: &str, fetch: F)
    where
        F: Fn(&str) -> Vec<u8>,
    {
        for prediction in self.predict(path, 5) {
            if cache.get(&prediction.key).is_none() {
                let data = fetch(&prediction.key);
                cache.put(&prediction.key, data);
            }
        }
    }
}

impl Default for PrefetchEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::horizon_cache::HorizonCacheConfig;

    #[test]
    fn predict_related_queries() {
        let engine = PrefetchEngine::new();
        let predictions = engine.predict("/accounts/GABC", 3);
        assert!(!predictions.is_empty());
        assert!(predictions[0].confidence > 0.0);
    }

    #[test]
    fn co_occurrence_boosts_confidence() {
        let mut engine = PrefetchEngine::new();
        engine.record_access("/accounts/A");
        engine.record_access("/accounts/A/payments");
        engine.record_access("/accounts/A");
        engine.record_access("/accounts/A/payments");

        let predictions = engine.predict("/accounts/A", 5);
        let payment_pred = predictions.iter().find(|p| p.key.contains("payments"));
        assert!(payment_pred.is_some());
    }
}
