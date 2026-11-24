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
/// Stellar-K8s Performance Benchmark Suite
///
/// Provides benchmarks for:
/// - CRD validation performance
/// - Helm template rendering
/// - Operator API throughput
/// - Reconciliation latency
/// - Network performance under load
///
/// Baselines are stored in benchmarks/baselines/ and tracked in CI for regression detection.

pub mod crd_validation {
    use serde::{Deserialize, Serialize};

    /// Benchmark: CRD YAML validation throughput
    ///
    /// Tests how many CRD manifests can be validated per second.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CrdValidationResult {
        pub total_manifests: usize,
        pub duration_secs: f64,
        pub throughput_per_sec: f64,
        pub average_validation_ms: f64,
        pub p95_validation_ms: f64,
        pub p99_validation_ms: f64,
    }

    impl CrdValidationResult {
        pub fn new(
            total: usize,
            duration: f64,
            timings: &[f64],
        ) -> Self {
            let throughput = total as f64 / duration;
            let avg = timings.iter().sum::<f64>() / timings.len() as f64;
            
            let mut sorted = timings.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            
            let p95_idx = (sorted.len() as f64 * 0.95) as usize;
            let p99_idx = (sorted.len() as f64 * 0.99) as usize;
            
            Self {
                total_manifests: total,
                duration_secs: duration,
                throughput_per_sec: throughput,
                average_validation_ms: avg * 1000.0,
                p95_validation_ms: sorted.get(p95_idx).unwrap_or(&0.0) * 1000.0,
                p99_validation_ms: sorted.get(p99_idx).unwrap_or(&0.0) * 1000.0,
            }
        }

        pub fn regression_check(&self, baseline: &Self, threshold_percent: f64) -> Result<(), String> {
            let allowed_regression = baseline.average_validation_ms * (threshold_percent / 100.0);
            let actual_increase = self.average_validation_ms - baseline.average_validation_ms;
            
            if actual_increase > allowed_regression {
                return Err(format!(
                    "CRD validation regression detected: {:.2}ms -> {:.2}ms (threshold: {:.1}%)",
                    baseline.average_validation_ms,
                    self.average_validation_ms,
                    threshold_percent
                ));
            }
            Ok(())
        }
    }
}

pub mod helm_rendering {
    use serde::{Deserialize, Serialize};

    /// Benchmark: Helm template rendering performance
    ///
    /// Measures time to render Helm charts with various values combinations.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct HelmRenderingResult {
        pub chart_name: String,
        pub values_count: usize,
        pub total_templates: usize,
        pub total_duration_secs: f64,
        pub average_per_template_ms: f64,
        pub p95_per_template_ms: f64,
        pub rendered_bytes: usize,
    }

    impl HelmRenderingResult {
        pub fn new(
            chart: &str,
            values: usize,
            templates: usize,
            duration: f64,
            timings: &[f64],
            rendered: usize,
        ) -> Self {
            let avg = timings.iter().sum::<f64>() / timings.len() as f64;
            
            let mut sorted = timings.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p95_idx = (sorted.len() as f64 * 0.95) as usize;
            
            Self {
                chart_name: chart.to_string(),
                values_count: values,
                total_templates: templates,
                total_duration_secs: duration,
                average_per_template_ms: avg * 1000.0,
                p95_per_template_ms: sorted.get(p95_idx).unwrap_or(&0.0) * 1000.0,
                rendered_bytes: rendered,
            }
        }

        pub fn regression_check(&self, baseline: &Self, threshold_percent: f64) -> Result<(), String> {
            let allowed_regression = baseline.average_per_template_ms * (threshold_percent / 100.0);
            let actual_increase = self.average_per_template_ms - baseline.average_per_template_ms;
            
            if actual_increase > allowed_regression {
                return Err(format!(
                    "Helm rendering regression: {:.2}ms -> {:.2}ms (threshold: {:.1}%)",
                    baseline.average_per_template_ms,
                    self.average_per_template_ms,
                    threshold_percent
                ));
            }
            Ok(())
        }
    }
}

pub mod operator_api {
    use serde::{Deserialize, Serialize};

    /// Benchmark: Operator REST API throughput under load
    ///
    /// Measures requests per second, latency percentiles, and error rates.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ApiThroughputResult {
        pub endpoint: String,
        pub total_requests: usize,
        pub successful_requests: usize,
        pub failed_requests: usize,
        pub duration_secs: f64,
        pub rps: f64,
        pub error_rate_percent: f64,
        pub avg_latency_ms: f64,
        pub p50_latency_ms: f64,
        pub p95_latency_ms: f64,
        pub p99_latency_ms: f64,
    }

    impl ApiThroughputResult {
        pub fn new(
            endpoint: &str,
            successful: usize,
            failed: usize,
            duration: f64,
            latencies: &[f64],
        ) -> Self {
            let total = successful + failed;
            let error_rate = (failed as f64 / total as f64) * 100.0;
            let rps = successful as f64 / duration;
            
            let mut sorted = latencies.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            
            let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
            let p50_idx = (sorted.len() as f64 * 0.50) as usize;
            let p95_idx = (sorted.len() as f64 * 0.95) as usize;
            let p99_idx = (sorted.len() as f64 * 0.99) as usize;
            
            Self {
                endpoint: endpoint.to_string(),
                total_requests: total,
                successful_requests: successful,
                failed_requests: failed,
                duration_secs: duration,
                rps,
                error_rate_percent: error_rate,
                avg_latency_ms: avg * 1000.0,
                p50_latency_ms: sorted.get(p50_idx).unwrap_or(&0.0) * 1000.0,
                p95_latency_ms: sorted.get(p95_idx).unwrap_or(&0.0) * 1000.0,
                p99_latency_ms: sorted.get(p99_idx).unwrap_or(&0.0) * 1000.0,
            }
        }

        pub fn regression_check(
            &self,
            baseline: &Self,
            latency_threshold_percent: f64,
        ) -> Result<(), String> {
            // Check latency regression
            let allowed_latency_increase = baseline.p99_latency_ms * (latency_threshold_percent / 100.0);
            let actual_latency_increase = self.p99_latency_ms - baseline.p99_latency_ms;
            
            if actual_latency_increase > allowed_latency_increase {
                return Err(format!(
                    "API latency regression: {:.2}ms -> {:.2}ms (threshold: {:.1}%)",
                    baseline.p99_latency_ms,
                    self.p99_latency_ms,
                    latency_threshold_percent
                ));
            }
            
            // Check throughput regression
            let min_throughput = baseline.rps * 0.95;  // Allow 5% throughput drop
            if self.rps < min_throughput {
                return Err(format!(
                    "API throughput regression: {:.0} -> {:.0} RPS",
                    baseline.rps,
                    self.rps
                ));
            }
            
            // Check error rate
            if self.error_rate_percent > baseline.error_rate_percent + 1.0 {
                return Err(format!(
                    "Error rate increased: {:.1}% -> {:.1}%",
                    baseline.error_rate_percent,
                    self.error_rate_percent
                ));
            }
            
            Ok(())
        }
    }
}

pub mod reconciliation {
    use serde::{Deserialize, Serialize};

    /// Benchmark: Operator reconciliation latency
    ///
    /// Measures time to complete reconciliation for different resource types.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ReconciliationResult {
        pub resource_type: String,
        pub resource_count: usize,
        pub total_reconciliations: usize,
        pub successful: usize,
        pub failed: usize,
        pub avg_duration_ms: f64,
        pub p50_duration_ms: f64,
        pub p95_duration_ms: f64,
        pub p99_duration_ms: f64,
        pub total_duration_secs: f64,
    }

    impl ReconciliationResult {
        pub fn new(
            resource_type: &str,
            count: usize,
            successful: usize,
            failed: usize,
            durations: &[f64],
        ) -> Self {
            let mut sorted = durations.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            
            let avg = durations.iter().sum::<f64>() / durations.len() as f64;
            let p50_idx = (sorted.len() as f64 * 0.50) as usize;
            let p95_idx = (sorted.len() as f64 * 0.95) as usize;
            let p99_idx = (sorted.len() as f64 * 0.99) as usize;
            
            let total = durations.iter().sum::<f64>();
            
            Self {
                resource_type: resource_type.to_string(),
                resource_count: count,
                total_reconciliations: successful + failed,
                successful,
                failed,
                avg_duration_ms: avg * 1000.0,
                p50_duration_ms: sorted.get(p50_idx).unwrap_or(&0.0) * 1000.0,
                p95_duration_ms: sorted.get(p95_idx).unwrap_or(&0.0) * 1000.0,
                p99_duration_ms: sorted.get(p99_idx).unwrap_or(&0.0) * 1000.0,
                total_duration_secs: total,
            }
        }

        pub fn regression_check(&self, baseline: &Self, threshold_percent: f64) -> Result<(), String> {
            let allowed_regression = baseline.p99_duration_ms * (threshold_percent / 100.0);
            let actual_increase = self.p99_duration_ms - baseline.p99_duration_ms;
            
            if actual_increase > allowed_regression {
                return Err(format!(
                    "Reconciliation latency regression: {:.2}ms -> {:.2}ms (threshold: {:.1}%)",
                    baseline.p99_duration_ms,
                    self.p99_duration_ms,
                    threshold_percent
                ));
            }
            Ok(())
        }
    }
}

pub mod metrics {
    use serde::{Deserialize, Serialize};

    /// Benchmark summary for CI reporting
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BenchmarkSummary {
        pub timestamp: String,
        pub git_commit: String,
        pub git_branch: String,
        pub duration_secs: f64,
        pub baseline_version: String,
        pub regressions: Vec<String>,
        pub warnings: Vec<String>,
    }

    impl BenchmarkSummary {
        pub fn has_regressions(&self) -> bool {
            !self.regressions.is_empty()
        }

        pub fn has_warnings(&self) -> bool {
            !self.warnings.is_empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crd_validation_regression_check() {
        let baseline = crd_validation::CrdValidationResult {
            total_manifests: 100,
            duration_secs: 1.0,
            throughput_per_sec: 100.0,
            average_validation_ms: 10.0,
            p95_validation_ms: 15.0,
            p99_validation_ms: 20.0,
        };

        // Test passing (within threshold)
        let passing = crd_validation::CrdValidationResult {
            total_manifests: 100,
            duration_secs: 1.05,
            throughput_per_sec: 95.0,
            average_validation_ms: 10.5,
            p95_validation_ms: 15.5,
            p99_validation_ms: 20.5,
        };
        assert!(passing.regression_check(&baseline, 10.0).is_ok());

        // Test failing (exceeds threshold)
        let failing = crd_validation::CrdValidationResult {
            total_manifests: 100,
            duration_secs: 2.0,
            throughput_per_sec: 50.0,
            average_validation_ms: 20.0,
            p95_validation_ms: 25.0,
            p99_validation_ms: 30.0,
        };
        assert!(failing.regression_check(&baseline, 10.0).is_err());
    }
}
