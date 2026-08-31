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
//! Profile analysis and bottleneck reporting (issue #1416).

use crate::profiling::collector::{AllocationSample, CpuSample};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Severity of an identified bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BottleneckSeverity {
    Info,
    Warning,
    Critical,
}

/// A single identified performance bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    /// Short identifier for this bottleneck type.
    pub id: String,
    pub severity: BottleneckSeverity,
    pub title: String,
    pub description: String,
    /// Suggested remediation.
    pub recommendation: String,
    /// Observed metric value that triggered this finding.
    pub observed_value: f64,
    /// Threshold that was breached.
    pub threshold: f64,
}

/// A complete analysis report over a set of profile samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckReport {
    /// Total CPU profiles analysed.
    pub cpu_samples_analysed: usize,
    /// Total heap profiles analysed.
    pub alloc_samples_analysed: usize,
    /// Identified bottlenecks, ordered by severity (critical first).
    pub bottlenecks: Vec<Bottleneck>,
    /// Top hot symbols across all CPU samples.
    pub top_symbols: Vec<SymbolHeat>,
    /// Trend: was RSS growing across samples?
    pub rss_growing: bool,
    /// Average RSS across samples (bytes).
    pub avg_rss_bytes: u64,
    /// Summary prose suitable for a runbook entry.
    pub summary: String,
}

/// Aggregated heat for a single symbol across CPU samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolHeat {
    pub symbol: String,
    pub total_samples: u64,
    pub pct: f64,
}

/// Analyses profile samples and produces bottleneck reports.
pub struct ProfileReporter {
    /// CPU wall-time threshold (ms) above which a warning is emitted.
    cpu_wall_warn_ms: f64,
    /// RSS threshold (bytes) above which a warning is emitted.
    rss_warn_bytes: u64,
    /// RSS growth rate (bytes/sample) that triggers a warning.
    rss_growth_warn_bytes_per_sample: u64,
}

impl Default for ProfileReporter {
    fn default() -> Self {
        Self {
            cpu_wall_warn_ms: 1_000.0,                          // >1 s
            rss_warn_bytes: 512 * 1024 * 1024,                  // >512 MB
            rss_growth_warn_bytes_per_sample: 10 * 1024 * 1024, // >10 MB/sample growth
        }
    }
}

impl ProfileReporter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyse the provided samples and return a [`BottleneckReport`].
    pub fn analyse(
        &self,
        cpu_samples: &[CpuSample],
        alloc_samples: &[AllocationSample],
    ) -> BottleneckReport {
        let mut bottlenecks: Vec<Bottleneck> = Vec::new();

        // ── CPU analysis ──────────────────────────────────────────────────────
        let mut symbol_totals: HashMap<String, u64> = HashMap::new();
        let mut total_all_samples: u64 = 0;

        for sample in cpu_samples {
            if sample.wall_time_ms > self.cpu_wall_warn_ms {
                bottlenecks.push(Bottleneck {
                    id: "high-cpu-wall-time".to_string(),
                    severity: BottleneckSeverity::Warning,
                    title: "High CPU wall-clock capture time".to_string(),
                    description: format!(
                        "CPU profile capture took {:.0} ms (threshold {:.0} ms). \
                         This may indicate CPU saturation or lock contention.",
                        sample.wall_time_ms, self.cpu_wall_warn_ms
                    ),
                    recommendation:
                        "Review hot functions below and consider async-offloading blocking calls."
                            .to_string(),
                    observed_value: sample.wall_time_ms,
                    threshold: self.cpu_wall_warn_ms,
                });
            }
            for (sym, &cnt) in &sample.stack_counts {
                *symbol_totals.entry(sym.clone()).or_insert(0) += cnt;
                total_all_samples += cnt;
            }
        }

        let mut top_symbols: Vec<SymbolHeat> = symbol_totals
            .iter()
            .map(|(sym, &cnt)| SymbolHeat {
                symbol: sym.clone(),
                total_samples: cnt,
                pct: if total_all_samples > 0 {
                    cnt as f64 / total_all_samples as f64 * 100.0
                } else {
                    0.0
                },
            })
            .collect();
        top_symbols.sort_by(|a, b| b.total_samples.cmp(&a.total_samples));
        top_symbols.truncate(10);

        // Warn if a single symbol dominates >50 % of samples
        if let Some(hot) = top_symbols.first() {
            if hot.pct > 50.0 {
                bottlenecks.push(Bottleneck {
                    id: "hot-function".to_string(),
                    severity: BottleneckSeverity::Critical,
                    title: format!("Hot function: {}", hot.symbol),
                    description: format!(
                        "`{}` accounts for {:.1}% of CPU samples. \
                         This is likely the primary bottleneck.",
                        hot.symbol, hot.pct
                    ),
                    recommendation:
                        "Profile with flamegraph, consider algorithmic improvements or caching."
                            .to_string(),
                    observed_value: hot.pct,
                    threshold: 50.0,
                });
            }
        }

        // ── Memory analysis ───────────────────────────────────────────────────
        let rss_values: Vec<u64> = alloc_samples.iter().map(|s| s.rss_bytes).collect();
        let avg_rss_bytes = if rss_values.is_empty() {
            0
        } else {
            rss_values.iter().sum::<u64>() / rss_values.len() as u64
        };

        let rss_growing = is_growing(&rss_values, self.rss_growth_warn_bytes_per_sample);

        if avg_rss_bytes > self.rss_warn_bytes {
            bottlenecks.push(Bottleneck {
                id: "high-rss".to_string(),
                severity: BottleneckSeverity::Warning,
                title: "High resident set size (RSS)".to_string(),
                description: format!(
                    "Average RSS is {:.0} MB (threshold {:.0} MB).",
                    avg_rss_bytes as f64 / (1024.0 * 1024.0),
                    self.rss_warn_bytes as f64 / (1024.0 * 1024.0)
                ),
                recommendation:
                    "Review top allocation sites; consider arena allocators or cache eviction."
                        .to_string(),
                observed_value: avg_rss_bytes as f64,
                threshold: self.rss_warn_bytes as f64,
            });
        }

        if rss_growing {
            bottlenecks.push(Bottleneck {
                id: "rss-growth".to_string(),
                severity: BottleneckSeverity::Critical,
                title: "RSS is growing monotonically — possible memory leak".to_string(),
                description: "RSS increased by more than the growth threshold on every sample. \
                               This is a strong indicator of a memory leak."
                    .to_string(),
                recommendation:
                    "Capture a heap profile and trace allocation sites with largest growth."
                        .to_string(),
                observed_value: avg_rss_bytes as f64,
                threshold: self.rss_growth_warn_bytes_per_sample as f64,
            });
        }

        // Sort: critical > warning > info
        bottlenecks.sort_by_key(|b| match b.severity {
            BottleneckSeverity::Critical => 0,
            BottleneckSeverity::Warning => 1,
            BottleneckSeverity::Info => 2,
        });

        let summary = build_summary(&bottlenecks, cpu_samples.len(), alloc_samples.len());

        BottleneckReport {
            cpu_samples_analysed: cpu_samples.len(),
            alloc_samples_analysed: alloc_samples.len(),
            bottlenecks,
            top_symbols,
            rss_growing,
            avg_rss_bytes,
            summary,
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` if each successive element grows by more than `threshold`.
fn is_growing(values: &[u64], threshold: u64) -> bool {
    if values.len() < 2 {
        return false;
    }
    values
        .windows(2)
        .all(|w| w[1].saturating_sub(w[0]) > threshold)
}

fn build_summary(bottlenecks: &[Bottleneck], cpu_count: usize, alloc_count: usize) -> String {
    if bottlenecks.is_empty() {
        format!(
            "No bottlenecks detected across {} CPU and {} heap profile(s). System appears healthy.",
            cpu_count, alloc_count
        )
    } else {
        let critical: Vec<&str> = bottlenecks
            .iter()
            .filter(|b| b.severity == BottleneckSeverity::Critical)
            .map(|b| b.title.as_str())
            .collect();
        let warnings: Vec<&str> = bottlenecks
            .iter()
            .filter(|b| b.severity == BottleneckSeverity::Warning)
            .map(|b| b.title.as_str())
            .collect();
        format!(
            "{} critical issue(s) and {} warning(s) found across {} CPU and {} heap sample(s). \
             Critical: {}. Warnings: {}.",
            critical.len(),
            warnings.len(),
            cpu_count,
            alloc_count,
            if critical.is_empty() {
                "none".to_string()
            } else {
                critical.join("; ")
            },
            if warnings.is_empty() {
                "none".to_string()
            } else {
                warnings.join("; ")
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn cpu_sample(wall_ms: f64, sym: &str, hits: u64) -> CpuSample {
        let mut map = HashMap::new();
        map.insert(sym.to_string(), hits);
        CpuSample {
            captured_at: Utc::now(),
            duration_secs: 10.0,
            stack_counts: map,
            cpu_time_ms: wall_ms,
            wall_time_ms: wall_ms,
            active_tasks: 1,
        }
    }

    fn alloc_sample(rss: u64) -> AllocationSample {
        AllocationSample {
            captured_at: Utc::now(),
            rss_bytes: rss,
            vsize_bytes: rss * 2,
            heap_allocated_bytes: rss + 1_000_000,
            heap_freed_bytes: 1_000_000,
            live_heap_bytes: rss,
            object_count: 1000,
            allocation_sites: HashMap::new(),
        }
    }

    #[test]
    fn empty_samples_produce_no_bottlenecks() {
        let reporter = ProfileReporter::new();
        let report = reporter.analyse(&[], &[]);
        assert!(report.bottlenecks.is_empty());
        assert!(report.summary.contains("No bottlenecks"));
    }

    #[test]
    fn high_wall_time_triggers_warning() {
        let reporter = ProfileReporter::new();
        let sample = cpu_sample(5_000.0, "slow_fn", 100);
        let report = reporter.analyse(&[sample], &[]);
        assert!(report
            .bottlenecks
            .iter()
            .any(|b| b.id == "high-cpu-wall-time"));
    }

    #[test]
    fn dominant_symbol_triggers_critical() {
        let reporter = ProfileReporter::new();
        let sample = cpu_sample(100.0, "hot_fn", 999);
        let report = reporter.analyse(&[sample], &[]);
        assert!(report
            .bottlenecks
            .iter()
            .any(|b| b.id == "hot-function" && b.severity == BottleneckSeverity::Critical));
    }

    #[test]
    fn growing_rss_triggers_critical() {
        let reporter = ProfileReporter::new();
        // Each sample grows by 20 MB, well above the 10 MB threshold.
        let samples: Vec<AllocationSample> = (0..3)
            .map(|i| alloc_sample(100 * 1024 * 1024 + i * 20 * 1024 * 1024))
            .collect();
        let report = reporter.analyse(&[], &samples);
        assert!(report.rss_growing);
        assert!(report.bottlenecks.iter().any(|b| b.id == "rss-growth"));
    }

    #[test]
    fn critical_bottlenecks_sorted_first() {
        let reporter = ProfileReporter::new();
        // Both warnings and a critical
        let sample = cpu_sample(5_000.0, "hot_fn", 9999);
        let alloc = alloc_sample(600 * 1024 * 1024); // above 512 MB threshold
        let report = reporter.analyse(&[sample], &[alloc]);
        if report.bottlenecks.len() >= 2 {
            assert_eq!(report.bottlenecks[0].severity, BottleneckSeverity::Critical);
        }
    }
}
