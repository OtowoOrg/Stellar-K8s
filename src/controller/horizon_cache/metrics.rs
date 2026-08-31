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
//! Prometheus metrics for Horizon query cache.

#[cfg(feature = "metrics")]
use once_cell::sync::Lazy;
#[cfg(feature = "metrics")]
use prometheus_client::encoding::EncodeLabelSet;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::counter::Counter;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::family::Family;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::gauge::Gauge;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
#[cfg(feature = "metrics")]
use std::sync::atomic::{AtomicI64, AtomicU64};

#[cfg(feature = "metrics")]
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct HorizonCacheLabels {
    pub namespace: String,
    pub node: String,
    pub layer: String,
}

#[cfg(feature = "metrics")]
pub static HORIZON_CACHE_HITS: Lazy<Family<HorizonCacheLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

#[cfg(feature = "metrics")]
pub static HORIZON_CACHE_MISSES: Lazy<Family<HorizonCacheLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

#[cfg(feature = "metrics")]
pub static HORIZON_CACHE_HIT_RATE: Lazy<Family<HorizonCacheLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

#[cfg(feature = "metrics")]
pub static HORIZON_QUERY_LATENCY: Lazy<Family<HorizonCacheLabels, Histogram>> = Lazy::new(|| {
    Family::new_with_constructor(|| Histogram::new(exponential_buckets(0.001, 2.0, 12)))
});

#[cfg(feature = "metrics")]
pub fn record_cache_hit(namespace: &str, node: &str, layer: &str) {
    let labels = HorizonCacheLabels {
        namespace: namespace.to_string(),
        node: node.to_string(),
        layer: layer.to_string(),
    };
    HORIZON_CACHE_HITS.get_or_create(&labels).inc();
}

#[cfg(feature = "metrics")]
pub fn record_cache_miss(namespace: &str, node: &str, layer: &str) {
    let labels = HorizonCacheLabels {
        namespace: namespace.to_string(),
        node: node.to_string(),
        layer: layer.to_string(),
    };
    HORIZON_CACHE_MISSES.get_or_create(&labels).inc();
}

#[cfg(feature = "metrics")]
pub fn record_hit_rate(namespace: &str, node: &str, layer: &str, rate_pct: f64) {
    let labels = HorizonCacheLabels {
        namespace: namespace.to_string(),
        node: node.to_string(),
        layer: layer.to_string(),
    };
    HORIZON_CACHE_HIT_RATE
        .get_or_create(&labels)
        .set(rate_pct as i64);
}

#[cfg(feature = "metrics")]
pub fn record_query_latency(namespace: &str, node: &str, layer: &str, seconds: f64) {
    let labels = HorizonCacheLabels {
        namespace: namespace.to_string(),
        node: node.to_string(),
        layer: layer.to_string(),
    };
    HORIZON_QUERY_LATENCY
        .get_or_create(&labels)
        .observe(seconds);
}

#[cfg(not(feature = "metrics"))]
pub fn record_cache_hit(_namespace: &str, _node: &str, _layer: &str) {}
#[cfg(not(feature = "metrics"))]
pub fn record_cache_miss(_namespace: &str, _node: &str, _layer: &str) {}
#[cfg(not(feature = "metrics"))]
pub fn record_hit_rate(_namespace: &str, _node: &str, _layer: &str, _rate_pct: f64) {}
#[cfg(not(feature = "metrics"))]
pub fn record_query_latency(_namespace: &str, _node: &str, _layer: &str, _seconds: f64) {}
