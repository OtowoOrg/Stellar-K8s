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
//! Intelligent rate-limiter and cache for the Prometheus metrics proxy.
//!
//! # Responsibilities
//!
//! 1. **Full-response rate-limiting** — enforces a minimum interval between
//!    upstream scrapes.  If Prometheus scrapes faster than the configured
//!    `min_scrape_interval`, the limiter returns the previous full response
//!    without touching the upstream node.  This is the primary defence against
//!    CPU spikes on high-throughput Stellar validators.
//!
//! 2. **Per-family metric caching** — slow-changing metrics (node version
//!    strings, uptime counters, static labels) can be pinned in a per-family
//!    TTL cache.  On a scrape, those families are served from cache while
//!    high-frequency metrics (SCP rounds, CPU, ledger close time) are always
//!    fetched fresh.
//!
//! 3. **High-frequency pass-through** — metrics matching the
//!    `high_frequency_patterns` list bypass the per-family cache entirely.
//!    This guarantees real-time visibility into consensus and resource metrics.
//!
//! # Memory budget
//!
//! Each cached value is a heap-allocated `String`.  For a typical Stellar Core
//! `/metrics` response (~200 KB), two full snapshots (current + previous) use
//! ~400 KB — well inside the 50 MB RAM constraint.  Per-family entries are
//! individually small (a few KB each).
//!
//! # Thread safety
//!
//! [`RateLimiter`] wraps all mutable state in a [`tokio::sync::Mutex`].
//! The lock is held only for the duration of a cache read/write, not during
//! the upstream HTTP fetch, so contention is minimal.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use regex::Regex;
use tokio::sync::Mutex;
use tracing::{debug, instrument};

use crate::proxy::config::{CacheRule, ProxyConfig};

// ---------------------------------------------------------------------------
// Internal state types
// ---------------------------------------------------------------------------

/// A cached Prometheus metric-family block: the raw text lines that belong to
/// one family (HELP + TYPE + data lines) and the time they were fetched.
#[derive(Clone)]
struct FamilyCache {
    text: String,
    fetched_at: Instant,
    ttl: Duration,
}

impl FamilyCache {
    fn is_stale(&self) -> bool {
        self.fetched_at.elapsed() > self.ttl
    }
}

/// Mutable inner state protected by a single async mutex.
struct Inner {
    /// Last full filtered response returned to Prometheus.
    last_full_response: Option<String>,
    /// When the last upstream scrape was performed.
    last_scrape_at: Option<Instant>,
    /// Per-family cache entries keyed by metric family name.
    family_cache: HashMap<String, FamilyCache>,
}

impl Inner {
    fn new() -> Self {
        Inner {
            last_full_response: None,
            last_scrape_at: None,
            family_cache: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Compiled rule sets
// ---------------------------------------------------------------------------

struct CompiledCacheRule {
    re: Regex,
    ttl: Duration,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Rate-limiter and cache engine.
///
/// Construct once from a [`ProxyConfig`] and share across Axum handler tasks
/// via `Arc<RateLimiter>`.
pub struct RateLimiter {
    min_scrape_interval: Duration,
    cache_rules: Vec<CompiledCacheRule>,
    high_frequency_res: Vec<Regex>,
    inner: Arc<Mutex<Inner>>,
}

impl RateLimiter {
    /// Build a [`RateLimiter`] from a [`ProxyConfig`].
    ///
    /// Returns an error if any regex pattern in `cache_rules` or
    /// `high_frequency_patterns` fails to compile.
    pub fn from_config(cfg: &ProxyConfig) -> Result<Self, regex::Error> {
        let cache_rules = compile_cache_rules(&cfg.cache_rules)?;
        let high_frequency_res = cfg
            .high_frequency_patterns
            .iter()
            .map(|p| Regex::new(p))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(RateLimiter {
            min_scrape_interval: cfg.min_scrape_interval(),
            cache_rules,
            high_frequency_res,
            inner: Arc::new(Mutex::new(Inner::new())),
        })
    }

    // -----------------------------------------------------------------------
    // Full-response rate-limiting
    // -----------------------------------------------------------------------

    /// Check whether a new upstream scrape is allowed or if the caller should
    /// use the cached full response.
    ///
    /// Returns `ScrapeDecision::UseCached(text)` if the last scrape was within
    /// `min_scrape_interval`, `ScrapeDecision::FetchFresh` otherwise.
    pub async fn check(&self) -> ScrapeDecision {
        let inner = self.inner.lock().await;
        if let (Some(last), Some(cached)) = (inner.last_scrape_at, &inner.last_full_response) {
            if last.elapsed() < self.min_scrape_interval {
                debug!(
                    elapsed_ms = last.elapsed().as_millis(),
                    min_ms = self.min_scrape_interval.as_millis(),
                    "rate-limit hit — returning cached full response"
                );
                return ScrapeDecision::UseCached(cached.clone());
            }
        }
        ScrapeDecision::FetchFresh
    }

    /// Store a freshly-fetched, fully-filtered response as the new cache entry
    /// and record the scrape timestamp.
    pub async fn store_full_response(&self, text: String) {
        let mut inner = self.inner.lock().await;
        inner.last_scrape_at = Some(Instant::now());
        inner.last_full_response = Some(text);
    }

    // -----------------------------------------------------------------------
    // Per-family caching
    // -----------------------------------------------------------------------

    /// Determine the TTL for a given metric family name.
    ///
    /// Returns `None` if no cache rule matches (i.e., the family is not
    /// eligible for caching) or if the family matches a high-frequency pattern
    /// (high-frequency metrics are never cached).
    pub fn cache_ttl_for(&self, family_name: &str) -> Option<Duration> {
        // High-frequency metrics are always fetched fresh.
        if self.is_high_frequency(family_name) {
            return None;
        }
        // Check cache rules in order; first match wins.
        for rule in &self.cache_rules {
            if rule.re.is_match(family_name) {
                return Some(rule.ttl);
            }
        }
        None
    }

    /// Returns `true` if the metric family should bypass the cache.
    pub fn is_high_frequency(&self, family_name: &str) -> bool {
        self.high_frequency_res
            .iter()
            .any(|re| re.is_match(family_name))
    }

    /// Look up a cached family block.
    ///
    /// Returns the cached text if an entry exists and has not yet expired.
    /// Expired entries are pruned on access.
    pub async fn get_family(&self, family_name: &str) -> Option<String> {
        let mut inner = self.inner.lock().await;
        if let Some(entry) = inner.family_cache.get(family_name) {
            if !entry.is_stale() {
                debug!(family_name, "family cache hit");
                return Some(entry.text.clone());
            }
            // Stale — remove.
            debug!(family_name, "family cache expired");
            inner.family_cache.remove(family_name);
        }
        None
    }

    /// Store a metric-family block in the per-family cache.
    ///
    /// `ttl` should come from [`RateLimiter::cache_ttl_for`].
    pub async fn store_family(&self, family_name: &str, text: String, ttl: Duration) {
        let mut inner = self.inner.lock().await;
        inner.family_cache.insert(
            family_name.to_owned(),
            FamilyCache {
                text,
                fetched_at: Instant::now(),
                ttl,
            },
        );
        debug!(family_name, ttl_secs = ttl.as_secs(), "family cached");
    }

    // -----------------------------------------------------------------------
    // Metrics for observability of the proxy itself
    // -----------------------------------------------------------------------

    /// Return a snapshot of current proxy cache statistics.
    pub async fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().await;
        CacheStats {
            cached_families: inner.family_cache.len(),
            has_full_response: inner.last_full_response.is_some(),
            last_scrape_age_ms: inner
                .last_scrape_at
                .map(|t| t.elapsed().as_millis() as u64),
        }
    }

    // -----------------------------------------------------------------------
    // Full-response assembly with per-family caching
    // -----------------------------------------------------------------------

    /// Split a raw Prometheus response into per-family text blocks and rebuild
    /// the response by serving cached blocks where possible and returning
    /// which families need to be re-fetched from upstream.
    ///
    /// Returns a `MergeResult` describing:
    /// - the families whose cached text can be reused,
    /// - the set of family names that are stale and must be fetched fresh.
    ///
    /// The caller is responsible for fetching fresh data for stale families and
    /// calling [`RateLimiter::store_family`] before assembling the final response.
    #[instrument(skip(self, raw_text), fields(response_bytes = raw_text.len()))]
    pub async fn split_and_merge(&self, raw_text: &str) -> MergeResult {
        let families = split_into_families(raw_text);
        let mut cached_blocks: Vec<String> = Vec::new();
        let mut stale_families: Vec<String> = Vec::new();

        for (name, block) in &families {
            // High-frequency: always mark stale so caller fetches fresh.
            if self.is_high_frequency(name) {
                stale_families.push(name.clone());
                cached_blocks.push(block.clone()); // use current block for now
                continue;
            }

            match self.cache_ttl_for(name) {
                None => {
                    // No cache rule — use the current block.
                    cached_blocks.push(block.clone());
                }
                Some(ttl) => {
                    if let Some(cached) = self.get_family(name).await {
                        cached_blocks.push(cached);
                    } else {
                        // Cache miss or expired — store current and mark stale
                        // for next cycle.
                        self.store_family(name, block.clone(), ttl).await;
                        cached_blocks.push(block.clone());
                        stale_families.push(name.clone());
                    }
                }
            }
        }

        MergeResult {
            assembled: cached_blocks.join(""),
            stale_family_names: stale_families,
        }
    }
}

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// Decision returned by [`RateLimiter::check`].
pub enum ScrapeDecision {
    /// The minimum scrape interval has not elapsed; use this cached body.
    UseCached(String),
    /// Enough time has passed — perform a fresh upstream scrape.
    FetchFresh,
}

/// Result of [`RateLimiter::split_and_merge`].
pub struct MergeResult {
    /// Assembled Prometheus text body (may mix cached and fresh blocks).
    pub assembled: String,
    /// Metric family names that were not served from cache this cycle.
    pub stale_family_names: Vec<String>,
}

/// Snapshot of proxy cache statistics, exposed via `/proxy-stats`.
#[derive(Debug, serde::Serialize)]
pub struct CacheStats {
    /// Number of metric families currently held in the per-family cache.
    pub cached_families: usize,
    /// Whether a full-response cache entry is present.
    pub has_full_response: bool,
    /// Milliseconds since the last upstream scrape, if any.
    pub last_scrape_age_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// Text splitting helper
// ---------------------------------------------------------------------------

/// Split a Prometheus text body into a list of `(family_name, block_text)`
/// pairs.
///
/// A "block" starts at `# HELP` (or `# TYPE` if no HELP is present) and
/// contains all associated data lines until the next block header or EOF.
/// Lines that don't belong to any block (stray comments, blank lines before
/// the first HELP) are assigned to a synthetic family named `__preamble__`.
pub fn split_into_families(text: &str) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    let mut current_family: Option<String> = None;
    let mut current_block = String::new();

    for line in text.lines() {
        if let Some(name) = help_family_name(line).or_else(|| {
            // Fall back to TYPE if there is no HELP (unusual but valid).
            if !line.starts_with("# HELP ") && current_family.is_none() {
                type_family_name(line)
            } else {
                None
            }
        }) {
            // Flush the previous block.
            if let Some(prev_name) = current_family.take() {
                if !current_block.is_empty() {
                    result.push((prev_name, current_block.clone()));
                    current_block.clear();
                }
            }
            current_family = Some(name.to_owned());
        }

        current_block.push_str(line);
        current_block.push('\n');
    }

    // Flush the last block.
    if let Some(name) = current_family {
        if !current_block.is_empty() {
            result.push((name, current_block));
        }
    } else if !current_block.trim().is_empty() {
        result.push(("__preamble__".to_owned(), current_block));
    }

    result
}

fn help_family_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("# HELP ")?;
    let end = rest
        .find(|c: char| c == ' ' || c == '\t')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn type_family_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("# TYPE ")?;
    let end = rest
        .find(|c: char| c == ' ' || c == '\t')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

// ---------------------------------------------------------------------------
// Compilation helpers
// ---------------------------------------------------------------------------

fn compile_cache_rules(rules: &[CacheRule]) -> Result<Vec<CompiledCacheRule>, regex::Error> {
    rules
        .iter()
        .map(|r| {
            Regex::new(&r.pattern).map(|re| CompiledCacheRule { re, ttl: r.ttl() })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::{CacheRule, ProxyConfig};
    use std::time::Duration;
    use tokio::time::sleep;

    fn make_limiter(min_interval_ms: u64, cache_rules: Vec<CacheRule>, hf: Vec<&str>) -> RateLimiter {
        let cfg = ProxyConfig {
            min_scrape_interval_ms: min_interval_ms,
            cache_rules,
            high_frequency_patterns: hf.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        };
        RateLimiter::from_config(&cfg).expect("valid test config")
    }

    // -----------------------------------------------------------------------
    // split_into_families
    // -----------------------------------------------------------------------

    #[test]
    fn split_two_families() {
        let text = "# HELP go_goroutines goroutines\n# TYPE go_goroutines gauge\ngo_goroutines 5\n\
                    # HELP up up status\n# TYPE up gauge\nup 1\n";
        let families = split_into_families(text);
        assert_eq!(families.len(), 2);
        assert_eq!(families[0].0, "go_goroutines");
        assert!(families[0].1.contains("go_goroutines 5"));
        assert_eq!(families[1].0, "up");
        assert!(families[1].1.contains("up 1"));
    }

    #[test]
    fn split_empty_returns_empty() {
        let families = split_into_families("");
        assert!(families.is_empty());
    }

    #[test]
    fn split_no_help_uses_type() {
        let text = "# TYPE up gauge\nup 1\n";
        let families = split_into_families(text);
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].0, "up");
    }

    // -----------------------------------------------------------------------
    // RateLimiter::check / store_full_response
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn first_check_returns_fetch_fresh() {
        let limiter = make_limiter(1000, vec![], vec![]);
        assert!(matches!(limiter.check().await, ScrapeDecision::FetchFresh));
    }

    #[tokio::test]
    async fn within_interval_returns_cached() {
        let limiter = make_limiter(10_000, vec![], vec![]);
        limiter.store_full_response("body".to_owned()).await;
        match limiter.check().await {
            ScrapeDecision::UseCached(body) => assert_eq!(body, "body"),
            ScrapeDecision::FetchFresh => panic!("expected cached response"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn after_interval_returns_fetch_fresh() {
        let limiter = make_limiter(50, vec![], vec![]);
        limiter.store_full_response("old".to_owned()).await;
        // Advance time past the interval.
        sleep(Duration::from_millis(100)).await;
        assert!(matches!(limiter.check().await, ScrapeDecision::FetchFresh));
    }

    // -----------------------------------------------------------------------
    // Per-family caching
    // -----------------------------------------------------------------------

    #[test]
    fn cache_ttl_for_matching_rule() {
        let limiter = make_limiter(
            1000,
            vec![CacheRule {
                pattern: "^stellar_node_version$".to_string(),
                ttl_secs: 300,
            }],
            vec![],
        );
        assert_eq!(
            limiter.cache_ttl_for("stellar_node_version"),
            Some(Duration::from_secs(300))
        );
    }

    #[test]
    fn cache_ttl_for_no_match_returns_none() {
        let limiter = make_limiter(1000, vec![], vec![]);
        assert_eq!(limiter.cache_ttl_for("go_goroutines"), None);
    }

    #[test]
    fn high_frequency_bypasses_cache_rule() {
        let limiter = make_limiter(
            1000,
            vec![CacheRule {
                pattern: "^stellar_scp_.*".to_string(),
                ttl_secs: 300,
            }],
            vec!["^stellar_scp_"],
        );
        // Even though a cache rule matches, high-frequency takes precedence.
        assert_eq!(limiter.cache_ttl_for("stellar_scp_rounds_total"), None);
    }

    #[tokio::test]
    async fn get_family_returns_none_on_miss() {
        let limiter = make_limiter(1000, vec![], vec![]);
        assert!(limiter.get_family("missing").await.is_none());
    }

    #[tokio::test]
    async fn get_family_returns_stored_value() {
        let limiter = make_limiter(1000, vec![], vec![]);
        limiter
            .store_family("up", "up 1\n".to_owned(), Duration::from_secs(60))
            .await;
        let val = limiter.get_family("up").await;
        assert_eq!(val.as_deref(), Some("up 1\n"));
    }

    #[tokio::test(start_paused = true)]
    async fn get_family_returns_none_after_ttl() {
        let limiter = make_limiter(1000, vec![], vec![]);
        limiter
            .store_family("up", "up 1\n".to_owned(), Duration::from_millis(50))
            .await;
        sleep(Duration::from_millis(100)).await;
        assert!(limiter.get_family("up").await.is_none());
    }

    // -----------------------------------------------------------------------
    // CacheStats
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn stats_empty_on_new_limiter() {
        let limiter = make_limiter(1000, vec![], vec![]);
        let stats = limiter.stats().await;
        assert_eq!(stats.cached_families, 0);
        assert!(!stats.has_full_response);
        assert!(stats.last_scrape_age_ms.is_none());
    }

    #[tokio::test]
    async fn stats_after_store() {
        let limiter = make_limiter(1000, vec![], vec![]);
        limiter.store_full_response("body".to_owned()).await;
        limiter
            .store_family("up", "up 1\n".to_owned(), Duration::from_secs(60))
            .await;
        let stats = limiter.stats().await;
        assert_eq!(stats.cached_families, 1);
        assert!(stats.has_full_response);
        assert!(stats.last_scrape_age_ms.is_some());
    }

    // -----------------------------------------------------------------------
    // from_config error handling
    // -----------------------------------------------------------------------

    #[test]
    fn from_config_invalid_cache_regex_returns_error() {
        let cfg = ProxyConfig {
            cache_rules: vec![CacheRule {
                pattern: "[invalid".to_string(),
                ttl_secs: 60,
            }],
            ..Default::default()
        };
        assert!(RateLimiter::from_config(&cfg).is_err());
    }

    #[test]
    fn from_config_invalid_hf_regex_returns_error() {
        let cfg = ProxyConfig {
            high_frequency_patterns: vec!["[bad".to_string()],
            ..Default::default()
        };
        assert!(RateLimiter::from_config(&cfg).is_err());
    }
}
