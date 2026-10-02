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
//! Criterion benchmarks for the metrics rate-limiter proxy.
//!
//! These benchmarks validate that the proxy meets its performance SLAs:
//!
//! - **Filter pipeline** (`filter/apply`): < 5 ms per 200 KB response.
//! - **Rate-limit hot path** (`limiter/cache_hit`): < 1 ms (async mutex + clone).
//! - **Family split** (`limiter/split_families`): < 2 ms per 200 KB response.
//!
//! Run with:
//!   cargo bench --bench proxy_throughput -p stellar-telemetry

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use stellar_telemetry::proxy::{
    config::{CacheRule, FilterAction, LabelDropRule, ProxyConfig, SeriesFilterRule},
    filter::MetricsFilter,
    limiter::{split_into_families, RateLimiter},
};

// ---------------------------------------------------------------------------
// Synthetic Prometheus response generator
// ---------------------------------------------------------------------------

/// Generate a synthetic Prometheus text response with `n_families` metric
/// families and `lines_per_family` data lines each.
fn synthetic_metrics(n_families: usize, lines_per_family: usize) -> String {
    let mut out = String::with_capacity(n_families * lines_per_family * 80);
    for i in 0..n_families {
        let name = format!("stellar_benchmark_metric_{i}");
        out.push_str(&format!("# HELP {name} Benchmark metric family {i}\n"));
        out.push_str(&format!("# TYPE {name} gauge\n"));
        for j in 0..lines_per_family {
            out.push_str(&format!(
                r#"{name}{{instance="pod-{j}",job="stellar",namespace="stellar-system"}} {val}"#,
                val = (i * 1000 + j) as f64 / 100.0
            ));
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Filter benchmarks
// ---------------------------------------------------------------------------

fn bench_filter_apply(c: &mut Criterion) {
    // Typical production config: drop instance/job labels, filter Go metrics.
    let config = ProxyConfig {
        label_drop_rules: vec![
            LabelDropRule { pattern: "^instance$".to_string() },
            LabelDropRule { pattern: "^job$".to_string() },
        ],
        series_filter_rules: vec![
            SeriesFilterRule {
                pattern: "^go_.*".to_string(),
                action: FilterAction::Drop,
            },
        ],
        ..Default::default()
    };
    let filter = MetricsFilter::from_config(&config).unwrap();

    let mut group = c.benchmark_group("filter/apply");

    for (families, lines) in [(50, 10), (100, 20), (200, 40)] {
        let input = synthetic_metrics(families, lines);
        let bytes = input.len();
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(
            BenchmarkId::new(format!("{families}f_{lines}l"), bytes),
            &input,
            |b, text| {
                b.iter(|| {
                    let _ = filter.apply(black_box(text));
                });
            },
        );
    }

    group.finish();
}

fn bench_filter_no_rules(c: &mut Criterion) {
    let filter = MetricsFilter::from_config(&ProxyConfig::default()).unwrap();

    let input = synthetic_metrics(100, 20);
    let bytes = input.len();

    let mut group = c.benchmark_group("filter/no_rules_passthrough");
    group.throughput(Throughput::Bytes(bytes as u64));
    group.bench_function("100f_20l", |b| {
        b.iter(|| {
            let _ = filter.apply(black_box(&input));
        });
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Limiter benchmarks
// ---------------------------------------------------------------------------

fn bench_limiter_cache_hit(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let config = ProxyConfig {
        min_scrape_interval_ms: 60_000, // ensure cache never expires
        ..Default::default()
    };
    let limiter = RateLimiter::from_config(&config).unwrap();

    // Pre-seed the cache with a realistic response body (~200 KB).
    let body = synthetic_metrics(100, 20);
    rt.block_on(limiter.store_full_response(body));

    let mut group = c.benchmark_group("limiter/cache_hit");
    group.bench_function("async_check", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = limiter.check().await;
        });
    });
    group.finish();
}

fn bench_limiter_split_families(c: &mut Criterion) {
    let mut group = c.benchmark_group("limiter/split_families");

    for (families, lines) in [(50, 10), (100, 20), (200, 40)] {
        let input = synthetic_metrics(families, lines);
        let bytes = input.len();
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(
            BenchmarkId::new(format!("{families}f_{lines}l"), bytes),
            &input,
            |b, text| {
                b.iter(|| {
                    let _ = split_into_families(black_box(text));
                });
            },
        );
    }

    group.finish();
}

fn bench_limiter_per_family_cache(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let config = ProxyConfig {
        cache_rules: vec![
            CacheRule { pattern: "^stellar_benchmark_metric_.*".to_string(), ttl_secs: 300 },
        ],
        min_scrape_interval_ms: 60_000,
        ..Default::default()
    };
    let limiter = RateLimiter::from_config(&config).unwrap();
    let body = synthetic_metrics(50, 5);

    // Pre-populate family cache.
    rt.block_on(async {
        for i in 0..50usize {
            limiter
                .store_family(
                    &format!("stellar_benchmark_metric_{i}"),
                    format!("# cached family {i}\n"),
                    std::time::Duration::from_secs(300),
                )
                .await;
        }
    });

    let mut group = c.benchmark_group("limiter/per_family_cache_lookup");
    group.bench_function("50_families_all_cached", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = limiter.split_and_merge(black_box(&body)).await;
        });
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// End-to-end latency benchmark
// ---------------------------------------------------------------------------

/// Simulates the complete hot-path: rate-limit check → return cached response.
/// This is the dominant path in production (most scrapes hit the cache).
fn bench_hot_path_end_to_end(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let config = ProxyConfig {
        min_scrape_interval_ms: 60_000,
        label_drop_rules: vec![
            LabelDropRule { pattern: "^instance$".to_string() },
            LabelDropRule { pattern: "^job$".to_string() },
        ],
        series_filter_rules: vec![SeriesFilterRule {
            pattern: "^go_.*".to_string(),
            action: FilterAction::Drop,
        }],
        ..Default::default()
    };

    let limiter = RateLimiter::from_config(&config).unwrap();
    let cached_body = synthetic_metrics(100, 20);
    rt.block_on(limiter.store_full_response(cached_body));

    let mut group = c.benchmark_group("e2e/hot_path_cache_hit");
    group.bench_function("rate_limit_check_and_return", |b| {
        b.to_async(&rt).iter(|| async {
            let decision = limiter.check().await;
            // Simulate returning the cached body without touching upstream.
            black_box(decision);
        });
    });
    group.finish();
}

criterion_group!(
    filter_benches,
    bench_filter_apply,
    bench_filter_no_rules,
);

criterion_group!(
    limiter_benches,
    bench_limiter_cache_hit,
    bench_limiter_split_families,
    bench_limiter_per_family_cache,
    bench_hot_path_end_to_end,
);

criterion_main!(filter_benches, limiter_benches);
