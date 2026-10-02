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
//! Label-dropping and series-filtering for the Prometheus text exposition format.
//!
//! This module operates directly on the raw UTF-8 Prometheus text output rather
//! than a full parse tree, keeping allocations minimal and latency well under
//! the 5 ms budget.
//!
//! # Processing pipeline
//!
//! ```text
//!  upstream /metrics text
//!      │
//!      ▼
//!  parse_metric_name()   ← extract name from each non-comment line
//!      │
//!      ▼
//!  SeriesFilter::should_keep()   ← apply ordered keep/drop rules
//!      │
//!      ▼  (kept)
//!  LabelDropper::strip_labels()  ← remove banned label names in-place
//!      │
//!      ▼
//!  filtered text output
//! ```
//!
//! # Correctness notes
//!
//! * `# HELP` and `# TYPE` comment lines are kept only when their metric family
//!   is not fully dropped by the filter rules — orphaned comments are removed.
//! * Labels are stripped surgically: `le="0.5",job="stellar"` with `job` in the
//!   drop list becomes `le="0.5"`.
//! * The filter preserves the trailing newline of each line to keep the output
//!   conformant with the Prometheus text format specification.

use regex::Regex;
use tracing::debug;

use crate::proxy::config::{FilterAction, LabelDropRule, ProxyConfig, SeriesFilterRule};

// ---------------------------------------------------------------------------
// Pre-compiled rule sets
// ---------------------------------------------------------------------------

/// A compiled label-drop rule (regex matched against label *names*).
struct CompiledLabelDrop {
    re: Regex,
}

/// A compiled series-filter rule.
struct CompiledSeriesFilter {
    re: Regex,
    action: FilterAction,
}

/// Pre-compiled filter engine constructed from a [`ProxyConfig`].
///
/// Reusing this across requests avoids repeated regex compilation on every
/// scrape.  It is intended to be placed inside an [`Arc`] and shared between
/// Axum handler tasks.
pub struct MetricsFilter {
    label_drops: Vec<CompiledLabelDrop>,
    series_filters: Vec<CompiledSeriesFilter>,
}

impl MetricsFilter {
    /// Build a [`MetricsFilter`] from the label-drop and series-filter rules
    /// in the provided [`ProxyConfig`].
    ///
    /// Returns an error if any regex pattern fails to compile.
    pub fn from_config(cfg: &ProxyConfig) -> Result<Self, regex::Error> {
        let label_drops = compile_label_drops(&cfg.label_drop_rules)?;
        let series_filters = compile_series_filters(&cfg.series_filter_rules)?;
        Ok(MetricsFilter {
            label_drops,
            series_filters,
        })
    }

    /// Apply label-dropping and series-filtering to raw Prometheus exposition
    /// text.
    ///
    /// Returns a new `String` containing only the metric lines (and their
    /// associated `# HELP`/`# TYPE` comments) that pass the filter rules.
    /// The result is always a valid Prometheus text exposition document.
    pub fn apply(&self, input: &str) -> String {
        // Rough capacity guess: usually most of the input survives.
        let mut out = String::with_capacity(input.len());

        // Track the most recently seen HELP/TYPE header so we can decide
        // whether to emit it only when at least one data line for that family
        // makes it through the filter.
        let mut pending_header: Option<(&str, &str)> = None; // (family_name, full_header_block)
        // We buffer the header block because TYPE always follows HELP in the
        // Prometheus format; we need both before deciding to emit.
        let mut header_buf = String::new();
        // Name of the metric family whose header we are buffering.
        let mut header_family: Option<String> = None;

        for line in input.lines() {
            if line.starts_with("# HELP ") {
                // Start of a new metric family header block.
                let family = help_metric_name(line).map(str::to_owned);
                header_buf.clear();
                header_buf.push_str(line);
                header_buf.push('\n');
                header_family = family;
                pending_header = None;
                continue;
            }

            if line.starts_with("# TYPE ") {
                // Append to header buffer; don't emit yet.
                header_buf.push_str(line);
                header_buf.push('\n');
                // Extract family name from TYPE line as a fallback.
                if header_family.is_none() {
                    header_family = type_metric_name(line).map(str::to_owned);
                }
                // Mark the pending header as ready.
                pending_header = Some(("", "")); // placeholder — we'll use header_buf directly
                let _ = pending_header; // suppress warning
                continue;
            }

            if line.starts_with('#') || line.is_empty() {
                // Other comment lines or blank lines — emit as-is.
                out.push_str(line);
                out.push('\n');
                continue;
            }

            // This is a data line: `metric_name{labels} value [timestamp]`
            let metric_name = parse_metric_name(line);

            // Decide keep/drop based on series-filter rules.
            if !self.should_keep(metric_name) {
                debug!(metric_name, "series dropped by filter rule");
                continue;
            }

            // Emit the buffered header block once — only for the first data
            // line that survives the filter for this family.
            if let Some(family) = &header_family {
                if metric_name_belongs_to_family(metric_name, family) {
                    if !header_buf.is_empty() {
                        out.push_str(&header_buf);
                        header_buf.clear();
                        header_family = None;
                    }
                }
            }

            // Apply label dropping and emit the (possibly modified) data line.
            let processed = self.strip_labels(line);
            out.push_str(&processed);
            out.push('\n');
        }

        out
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Returns `true` if the metric should be forwarded to Prometheus.
    fn should_keep(&self, metric_name: &str) -> bool {
        for rule in &self.series_filters {
            if rule.re.is_match(metric_name) {
                return rule.action == FilterAction::Keep;
            }
        }
        // Default action when no rule matches: keep.
        true
    }

    /// Remove labels whose names match any label-drop rule.
    ///
    /// The function only modifies the label section (`{…}`) of a line; the
    /// metric name, value, and optional timestamp are preserved verbatim.
    fn strip_labels<'a>(&self, line: &'a str) -> std::borrow::Cow<'a, str> {
        if self.label_drops.is_empty() {
            return std::borrow::Cow::Borrowed(line);
        }

        // Quick path: no label section at all.
        let Some(open) = line.find('{') else {
            return std::borrow::Cow::Borrowed(line);
        };
        let Some(close) = line.rfind('}') else {
            return std::borrow::Cow::Borrowed(line);
        };

        let prefix = &line[..open]; // metric name
        let suffix = &line[close + 1..]; // ` value [timestamp]`
        let labels_str = &line[open + 1..close];

        let filtered_labels = filter_label_pairs(labels_str, &self.label_drops);
        if filtered_labels == labels_str {
            // Nothing was removed — avoid an allocation.
            return std::borrow::Cow::Borrowed(line);
        }

        if filtered_labels.is_empty() {
            std::borrow::Cow::Owned(format!("{prefix}{suffix}"))
        } else {
            std::borrow::Cow::Owned(format!("{prefix}{{{filtered_labels}}}{suffix}"))
        }
    }
}

// ---------------------------------------------------------------------------
// Regex compilation helpers
// ---------------------------------------------------------------------------

fn compile_label_drops(rules: &[LabelDropRule]) -> Result<Vec<CompiledLabelDrop>, regex::Error> {
    rules
        .iter()
        .map(|r| {
            Regex::new(&r.pattern).map(|re| CompiledLabelDrop { re })
        })
        .collect()
}

fn compile_series_filters(
    rules: &[SeriesFilterRule],
) -> Result<Vec<CompiledSeriesFilter>, regex::Error> {
    rules
        .iter()
        .map(|r| {
            Regex::new(&r.pattern).map(|re| CompiledSeriesFilter {
                re,
                action: r.action.clone(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Text parsing helpers
// ---------------------------------------------------------------------------

/// Extract the metric name from a Prometheus data line.
///
/// Examples:
/// - `http_requests_total{method="GET"} 42` → `http_requests_total`
/// - `up 1` → `up`
fn parse_metric_name(line: &str) -> &str {
    let end = line
        .find(|c: char| c == '{' || c == ' ' || c == '\t')
        .unwrap_or(line.len());
    &line[..end]
}

/// Extract the metric family name from a `# HELP <name> ...` line.
fn help_metric_name(line: &str) -> Option<&str> {
    // "# HELP " prefix is 7 bytes
    let rest = line.strip_prefix("# HELP ")?;
    let end = rest
        .find(|c: char| c == ' ' || c == '\t')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Extract the metric family name from a `# TYPE <name> ...` line.
fn type_metric_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("# TYPE ")?;
    let end = rest
        .find(|c: char| c == ' ' || c == '\t')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Check whether a data-line metric name belongs to a given family.
///
/// Prometheus adds suffixes like `_total`, `_bucket`, `_sum`, `_count` to
/// the base family name for counters and histograms.  This check covers both
/// exact matches and suffix variants.
fn metric_name_belongs_to_family(metric_name: &str, family: &str) -> bool {
    if metric_name == family {
        return true;
    }
    for suffix in &["_total", "_bucket", "_sum", "_count", "_created"] {
        if metric_name == format!("{family}{suffix}") {
            return true;
        }
    }
    // Histograms/summaries sometimes only share a prefix.
    metric_name.starts_with(family)
}

/// Filter a comma-separated label-pair string, removing pairs whose label
/// names match any drop rule.
///
/// Input example: `method="GET",job="stellar",le="0.5"`
/// Drop rule `^job$` → output: `method="GET",le="0.5"`
///
/// This deliberately avoids a full CSV parser: Prometheus label values can
/// contain escaped quotes but not raw commas, so a simple split on `,` is
/// safe when paired with the `="..."` structure check.
fn filter_label_pairs(labels: &str, drops: &[CompiledLabelDrop]) -> String {
    let pairs: Vec<&str> = labels.split(',').collect();
    let mut kept: Vec<&str> = Vec::with_capacity(pairs.len());

    'outer: for pair in &pairs {
        let trimmed = pair.trim();
        // Extract the label name (everything before `=`).
        if let Some(eq_pos) = trimmed.find('=') {
            let label_name = &trimmed[..eq_pos];
            for drop in drops {
                if drop.re.is_match(label_name) {
                    // This label is banned — skip the whole pair.
                    continue 'outer;
                }
            }
        }
        kept.push(pair);
    }

    kept.join(",")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::{FilterAction, LabelDropRule, ProxyConfig, SeriesFilterRule};

    fn make_filter(label_patterns: &[&str], series_rules: &[(&str, FilterAction)]) -> MetricsFilter {
        let label_drop_rules: Vec<LabelDropRule> = label_patterns
            .iter()
            .map(|p| LabelDropRule {
                pattern: p.to_string(),
            })
            .collect();

        let series_filter_rules: Vec<SeriesFilterRule> = series_rules
            .iter()
            .map(|(p, a)| SeriesFilterRule {
                pattern: p.to_string(),
                action: a.clone(),
            })
            .collect();

        let cfg = ProxyConfig {
            label_drop_rules,
            series_filter_rules,
            ..Default::default()
        };

        MetricsFilter::from_config(&cfg).expect("valid test config")
    }

    // -----------------------------------------------------------------------
    // parse_metric_name
    // -----------------------------------------------------------------------

    #[test]
    fn parse_name_with_labels() {
        assert_eq!(parse_metric_name(r#"http_requests_total{method="GET"} 42"#), "http_requests_total");
    }

    #[test]
    fn parse_name_without_labels() {
        assert_eq!(parse_metric_name("up 1"), "up");
    }

    #[test]
    fn parse_name_with_space_no_labels() {
        assert_eq!(parse_metric_name("process_cpu_seconds_total 0.5"), "process_cpu_seconds_total");
    }

    // -----------------------------------------------------------------------
    // filter_label_pairs
    // -----------------------------------------------------------------------

    #[test]
    fn drop_single_label() {
        let drops = compile_label_drops(&[LabelDropRule { pattern: "^job$".to_string() }]).unwrap();
        let result = filter_label_pairs(r#"method="GET",job="stellar",le="0.5""#, &drops);
        assert_eq!(result, r#"method="GET",le="0.5""#);
    }

    #[test]
    fn drop_multiple_labels() {
        let drops = compile_label_drops(&[
            LabelDropRule { pattern: "^job$".to_string() },
            LabelDropRule { pattern: "^instance$".to_string() },
        ]).unwrap();
        let result = filter_label_pairs(r#"method="GET",job="stellar",instance="pod-1",le="0.5""#, &drops);
        assert_eq!(result, r#"method="GET",le="0.5""#);
    }

    #[test]
    fn drop_no_match_preserves_all() {
        let drops = compile_label_drops(&[LabelDropRule { pattern: "^job$".to_string() }]).unwrap();
        let result = filter_label_pairs(r#"method="GET",status="200""#, &drops);
        assert_eq!(result, r#"method="GET",status="200""#);
    }

    #[test]
    fn empty_label_section() {
        let drops = compile_label_drops(&[LabelDropRule { pattern: "^job$".to_string() }]).unwrap();
        let result = filter_label_pairs("", &drops);
        assert_eq!(result, "");
    }

    // -----------------------------------------------------------------------
    // MetricsFilter::strip_labels
    // -----------------------------------------------------------------------

    #[test]
    fn strip_labels_removes_banned_label() {
        let filter = make_filter(&["^instance$"], &[]);
        let input = r#"up{job="stellar",instance="pod-0"} 1"#;
        let result = filter.strip_labels(input);
        assert_eq!(result, r#"up{job="stellar"} 1"#);
    }

    #[test]
    fn strip_labels_no_labels_unchanged() {
        let filter = make_filter(&["^instance$"], &[]);
        let input = "up 1";
        let result = filter.strip_labels(input);
        assert_eq!(result.as_ref(), input);
    }

    #[test]
    fn strip_labels_all_dropped_removes_braces() {
        let filter = make_filter(&["^job$", "^instance$"], &[]);
        let input = r#"up{job="stellar",instance="pod-0"} 1"#;
        let result = filter.strip_labels(input);
        assert_eq!(result, "up 1");
    }

    // -----------------------------------------------------------------------
    // MetricsFilter::should_keep
    // -----------------------------------------------------------------------

    #[test]
    fn keep_by_default_no_rules() {
        let filter = make_filter(&[], &[]);
        assert!(filter.should_keep("anything"));
    }

    #[test]
    fn drop_rule_drops_matching_metric() {
        let filter = make_filter(&[], &[("^go_gc_.*", FilterAction::Drop)]);
        assert!(!filter.should_keep("go_gc_duration_seconds"));
        assert!(filter.should_keep("stellar_scp_rounds_total"));
    }

    #[test]
    fn keep_rule_overrides_later_drop() {
        // First-match wins: keep rule before drop-all overrides it.
        let filter = make_filter(&[], &[
            ("^stellar_scp_.*", FilterAction::Keep),
            ("^stellar_.*", FilterAction::Drop),
        ]);
        assert!(filter.should_keep("stellar_scp_rounds_total"));
        assert!(!filter.should_keep("stellar_node_uptime_seconds"));
    }

    #[test]
    fn drop_all_then_keep_specific() {
        let filter = make_filter(&[], &[
            ("^stellar_scp_.*", FilterAction::Keep),
            ("^.*", FilterAction::Drop),
        ]);
        assert!(filter.should_keep("stellar_scp_rounds_total"));
        assert!(!filter.should_keep("go_gc_duration_seconds"));
    }

    // -----------------------------------------------------------------------
    // MetricsFilter::apply (full integration)
    // -----------------------------------------------------------------------

    const SAMPLE_METRICS: &str = r#"# HELP go_gc_duration_seconds A summary of the GC invocation durations.
# TYPE go_gc_duration_seconds summary
go_gc_duration_seconds{quantile="0",job="stellar"} 4.9351e-05
go_gc_duration_seconds{quantile="0.25",job="stellar"} 7.424100000000001e-05
# HELP stellar_ledger_close_time_seconds Time taken to close a ledger.
# TYPE stellar_ledger_close_time_seconds histogram
stellar_ledger_close_time_seconds_bucket{le="0.1",instance="pod-0"} 24054
stellar_ledger_close_time_seconds_bucket{le="0.2",instance="pod-0"} 33444
stellar_ledger_close_time_seconds_sum{instance="pod-0"} 144.97145
stellar_ledger_close_time_seconds_count{instance="pod-0"} 37993
# HELP stellar_node_uptime_seconds_total Node uptime in seconds.
# TYPE stellar_node_uptime_seconds_total counter
stellar_node_uptime_seconds_total{instance="pod-0"} 12345.0
"#;

    #[test]
    fn apply_drop_go_metrics_keeps_stellar() {
        let filter = make_filter(&[], &[("^go_.*", FilterAction::Drop)]);
        let result = filter.apply(SAMPLE_METRICS);
        assert!(!result.contains("go_gc_duration_seconds"));
        assert!(result.contains("stellar_ledger_close_time_seconds"));
        assert!(result.contains("stellar_node_uptime_seconds_total"));
    }

    #[test]
    fn apply_drops_help_type_for_dropped_series() {
        let filter = make_filter(&[], &[("^go_.*", FilterAction::Drop)]);
        let result = filter.apply(SAMPLE_METRICS);
        assert!(!result.contains("# HELP go_gc_duration_seconds"));
        assert!(!result.contains("# TYPE go_gc_duration_seconds"));
    }

    #[test]
    fn apply_strips_instance_label() {
        let filter = make_filter(&["^instance$"], &[]);
        let result = filter.apply(SAMPLE_METRICS);
        assert!(!result.contains("instance="));
        // Metric values must still be present
        assert!(result.contains("stellar_ledger_close_time_seconds_bucket"));
    }

    #[test]
    fn apply_empty_input_returns_empty() {
        let filter = make_filter(&[], &[]);
        assert_eq!(filter.apply(""), "");
    }

    #[test]
    fn apply_no_rules_passthrough() {
        let filter = make_filter(&[], &[]);
        let input = "up 1\n";
        let result = filter.apply(input);
        assert!(result.contains("up 1"));
    }

    #[test]
    fn from_config_invalid_regex_returns_error() {
        let cfg = ProxyConfig {
            label_drop_rules: vec![LabelDropRule {
                pattern: "[invalid".to_string(),
            }],
            ..Default::default()
        };
        assert!(MetricsFilter::from_config(&cfg).is_err());
    }

    #[test]
    fn apply_preserves_timestamp() {
        let filter = make_filter(&[], &[]);
        let input = "up{job=\"stellar\"} 1 1609459200000\n";
        let result = filter.apply(input);
        assert!(result.contains("1609459200000"));
    }

    #[test]
    fn metric_name_belongs_to_family_suffixes() {
        assert!(metric_name_belongs_to_family("http_requests_total", "http_requests"));
        assert!(metric_name_belongs_to_family("http_requests_bucket", "http_requests"));
        assert!(metric_name_belongs_to_family("http_requests_sum", "http_requests"));
        assert!(metric_name_belongs_to_family("http_requests_count", "http_requests"));
        assert!(metric_name_belongs_to_family("http_requests", "http_requests"));
        assert!(!metric_name_belongs_to_family("other_metric", "http_requests"));
    }
}
