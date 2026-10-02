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
//! Configuration for the Prometheus metrics rate-limiter proxy.
//!
//! Loaded from the file specified by `METRICS_PROXY_CONFIG` (default:
//! `/etc/stellar-metrics-proxy/config.yaml`).  The file is a YAML document
//! that is typically mounted from a Kubernetes ConfigMap.
//!
//! # Example ConfigMap content
//!
//! ```yaml
//! upstream: "http://stellar-core:11626/metrics"
//! listenAddr: "0.0.0.0:9091"
//! labelDropRules:
//!   - pattern: "^le$"
//!   - pattern: "^quantile$"
//!   - pattern: "^instance$"
//! seriesFilterRules:
//!   - pattern: "^go_gc_.*"
//!     action: drop
//!   - pattern: "^stellar_scp_.*"
//!     action: keep
//! cacheRules:
//!   - pattern: "^stellar_node_version$"
//!     ttlSecs: 300
//!   - pattern: "^stellar_node_uptime_seconds_total$"
//!     ttlSecs: 60
//! highFrequencyPatterns:
//!   - "^stellar_scp_"
//!   - "^stellar_ledger_"
//!   - "^process_cpu_"
//! minScrapeIntervalMs: 1000
//! ```

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

/// Default path for the proxy configuration file (mounted from a ConfigMap).
const DEFAULT_CONFIG_PATH: &str = "/etc/stellar-metrics-proxy/config.yaml";

/// Action to apply when a series-filter rule matches a metric name.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FilterAction {
    /// Drop the metric series entirely — do not forward it to Prometheus.
    Drop,
    /// Explicitly keep the metric series (overrides a blanket drop policy).
    Keep,
}

impl Default for FilterAction {
    fn default() -> Self {
        FilterAction::Drop
    }
}

/// A label-drop rule.
///
/// Any label whose name matches `pattern` is stripped from every metric line
/// before the response is forwarded to Prometheus.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LabelDropRule {
    /// Regex pattern matched against label *names* (anchored to the full name
    /// by default — wrap in `.*` for substring matching).
    pub pattern: String,
}

/// A series-filter rule.
///
/// Rules are evaluated in order; the first match wins.  Unmatched metrics
/// fall through with the implicit default action (`keep`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesFilterRule {
    /// Regex pattern matched against the metric *name* (the part before `{`).
    pub pattern: String,

    /// What to do when the pattern matches.
    #[serde(default)]
    pub action: FilterAction,
}

/// A cache rule for slow-changing metrics.
///
/// When a metric name matches `pattern`, its scraped value is cached for
/// `ttl_secs` seconds.  Subsequent scrapes within the TTL window return the
/// cached value without fetching it from upstream, reducing upstream CPU load.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheRule {
    /// Regex pattern matched against the metric *name*.
    pub pattern: String,

    /// How long to cache this metric family before fetching fresh data.
    #[serde(default = "default_ttl_secs")]
    pub ttl_secs: u64,
}

impl CacheRule {
    /// Return the TTL as a [`Duration`].
    pub fn ttl(&self) -> Duration {
        Duration::from_secs(self.ttl_secs)
    }
}

fn default_ttl_secs() -> u64 {
    60
}

/// Top-level proxy configuration schema.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    /// Full URL of the upstream Stellar Core `/metrics` endpoint.
    #[serde(default = "default_upstream")]
    pub upstream: String,

    /// Address and port this proxy server listens on.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// Rules specifying which label *names* to strip from every metric line.
    #[serde(default)]
    pub label_drop_rules: Vec<LabelDropRule>,

    /// Rules controlling which metric series are forwarded (`keep`) or
    /// suppressed (`drop`).  Evaluated in order; first match wins.
    #[serde(default)]
    pub series_filter_rules: Vec<SeriesFilterRule>,

    /// Caching rules for slow-changing metric families.
    #[serde(default)]
    pub cache_rules: Vec<CacheRule>,

    /// Regex patterns for metric names that should bypass the cache and always
    /// be fetched fresh on every scrape (e.g., CPU, consensus round metrics).
    #[serde(default)]
    pub high_frequency_patterns: Vec<String>,

    /// Minimum interval (milliseconds) between upstream scrapes.
    /// Prometheus scrapes arriving sooner than this return the last cached full
    /// response.  This prevents CPU spikes from concurrent rapid scrapes.
    ///
    /// Default: 1 000 ms (1 second).
    #[serde(default = "default_min_scrape_interval_ms")]
    pub min_scrape_interval_ms: u64,

    /// HTTP request timeout for fetching the upstream `/metrics` endpoint.
    ///
    /// Default: 10 000 ms (10 seconds).
    #[serde(default = "default_upstream_timeout_ms")]
    pub upstream_timeout_ms: u64,
}

fn default_upstream() -> String {
    "http://127.0.0.1:11626/metrics".to_string()
}

fn default_listen_addr() -> String {
    "0.0.0.0:9091".to_string()
}

fn default_min_scrape_interval_ms() -> u64 {
    1_000
}

fn default_upstream_timeout_ms() -> u64 {
    10_000
}

impl Default for ProxyConfig {
    fn default() -> Self {
        ProxyConfig {
            upstream: default_upstream(),
            listen_addr: default_listen_addr(),
            label_drop_rules: Vec::new(),
            series_filter_rules: Vec::new(),
            cache_rules: Vec::new(),
            high_frequency_patterns: Vec::new(),
            min_scrape_interval_ms: default_min_scrape_interval_ms(),
            upstream_timeout_ms: default_upstream_timeout_ms(),
        }
    }
}

impl ProxyConfig {
    /// Load config from the path given by `METRICS_PROXY_CONFIG` or the
    /// default path.  Returns `Default::default()` if the file is absent or
    /// cannot be parsed.
    pub fn load() -> Self {
        let path = std::env::var("METRICS_PROXY_CONFIG")
            .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        Self::load_from_file(&path)
    }

    /// Load config from an explicit file path.
    pub fn load_from_file(path: &str) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                tracing::debug!(
                    "No metrics-proxy config found at {path}. Using built-in defaults."
                );
                return Self::default();
            }
        };

        match serde_yaml::from_str::<ProxyConfig>(&contents) {
            Ok(cfg) => {
                tracing::info!("Loaded metrics-proxy config from {path}");
                cfg
            }
            Err(e) => {
                warn!("Failed to parse metrics-proxy config at {path}: {e}. Using defaults.");
                Self::default()
            }
        }
    }

    /// Convenience: return the minimum scrape interval as a [`Duration`].
    pub fn min_scrape_interval(&self) -> Duration {
        Duration::from_millis(self.min_scrape_interval_ms)
    }

    /// Convenience: return the upstream request timeout as a [`Duration`].
    pub fn upstream_timeout(&self) -> Duration {
        Duration::from_millis(self.upstream_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_is_valid() {
        let cfg = ProxyConfig::default();
        assert_eq!(cfg.upstream, "http://127.0.0.1:11626/metrics");
        assert_eq!(cfg.listen_addr, "0.0.0.0:9091");
        assert_eq!(cfg.min_scrape_interval_ms, 1_000);
        assert_eq!(cfg.upstream_timeout_ms, 10_000);
        assert!(cfg.label_drop_rules.is_empty());
        assert!(cfg.series_filter_rules.is_empty());
        assert!(cfg.cache_rules.is_empty());
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let cfg = ProxyConfig::load_from_file("/nonexistent/path/config.yaml");
        assert_eq!(cfg.upstream, "http://127.0.0.1:11626/metrics");
    }

    #[test]
    fn load_from_valid_yaml() {
        let yaml = r#"
upstream: "http://stellar-core:11626/metrics"
listenAddr: "0.0.0.0:9091"
minScrapeIntervalMs: 2000
upstreamTimeoutMs: 5000
labelDropRules:
  - pattern: "^le$"
  - pattern: "^instance$"
seriesFilterRules:
  - pattern: "^go_gc_.*"
    action: drop
  - pattern: "^stellar_scp_.*"
    action: keep
cacheRules:
  - pattern: "^stellar_node_version$"
    ttlSecs: 300
highFrequencyPatterns:
  - "^stellar_scp_"
  - "^process_cpu_"
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();

        let cfg = ProxyConfig::load_from_file(f.path().to_str().unwrap());
        assert_eq!(cfg.upstream, "http://stellar-core:11626/metrics");
        assert_eq!(cfg.min_scrape_interval_ms, 2000);
        assert_eq!(cfg.label_drop_rules.len(), 2);
        assert_eq!(cfg.series_filter_rules.len(), 2);
        assert_eq!(cfg.series_filter_rules[0].action, FilterAction::Drop);
        assert_eq!(cfg.cache_rules.len(), 1);
        assert_eq!(cfg.cache_rules[0].ttl_secs, 300);
        assert_eq!(cfg.high_frequency_patterns.len(), 2);
    }

    #[test]
    fn load_from_invalid_yaml_returns_default() {
        let yaml = "this: is: not: valid: yaml: [[[";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();

        let cfg = ProxyConfig::load_from_file(f.path().to_str().unwrap());
        assert_eq!(cfg.upstream, "http://127.0.0.1:11626/metrics");
    }

    #[test]
    fn ttl_duration_conversion() {
        let rule = CacheRule {
            pattern: "^test$".to_string(),
            ttl_secs: 120,
        };
        assert_eq!(rule.ttl(), Duration::from_secs(120));
    }

    #[test]
    fn min_scrape_interval_duration_conversion() {
        let cfg = ProxyConfig {
            min_scrape_interval_ms: 500,
            ..Default::default()
        };
        assert_eq!(cfg.min_scrape_interval(), Duration::from_millis(500));
    }
}
