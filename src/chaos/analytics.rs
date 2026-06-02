//! Chaos Engineering Framework - Analytics and Reporting
//!
//! Provides experiment tracking, metrics, and analysis.

use chrono::{DateTime, Utc, Duration as ChronoDuration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Experiment run record for analytics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentRun {
    pub id: String,
    pub name: String,
    pub namespace: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub phase: String,
    pub success: bool,
    pub results: Option<ExperimentResults>,
    pub faults: Vec<FaultExecution>,
    pub probe_results: Vec<ProbeResult>,
}

/// Aggregated chaos metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosMetrics {
    pub total_experiments: u64,
    pub successful_experiments: u64,
    pub failed_experiments: u64,
    pub success_rate: f32,

    pub total_faults_injected: u64,
    pub total_faults_recovered: u64,
    pub recovery_rate: f32,

    pub avg_resilience_score: f32,
    pub avg_availability: f32,
    pub avg_error_rate: f32,

    pub experiments_by_type: HashMap<String, u64>,
    pub faults_by_type: HashMap<String, u64>,

    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

/// Experiment summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentSummary {
    pub total_runs: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_duration_seconds: f64,
    pub avg_resilience_score: f32,

    pub most_common_failures: Vec<FailureInfo>,
    pub improvement_trend: TrendDirection,
}

/// Failure information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureInfo {
    pub failure_type: String,
    pub count: u64,
    pub percentage: f32,
    pub avg_recovery_time_ms: u64,
}

/// Trend direction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Improving,
    Stable,
    Degrading,
}

/// Chaos report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosReport {
    pub id: String,
    pub generated_at: DateTime<Utc>,
    pub period: ReportPeriod,
    pub metrics: ChaosMetrics,
    pub experiment_summaries: Vec<ExperimentSummary>,
    pub recommendations: Vec<String>,
    pub system_resilience_score: u8,
}

/// Report period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportPeriod {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub granularity: ReportGranularity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportGranularity {
    Hourly,
    Daily,
    Weekly,
    Monthly,
}

/// Chaos analytics engine
pub struct ChaosAnalytics {
    runs: Vec<ExperimentRun>,
    max_runs: usize,
}

impl ChaosAnalytics {
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            max_runs: 10000,
        }
    }

    /// Record an experiment run
    pub fn record(&mut self, run: ExperimentRun) {
        if self.runs.len() >= self.max_runs {
            self.runs.remove(0);
        }
        self.runs.push(run);
    }

    /// Get metrics for a time period
    pub fn get_metrics(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> ChaosMetrics {
        let runs_in_period: Vec<&ExperimentRun> = self.runs.iter()
            .filter(|r| r.started_at >= start && r.started_at <= end)
            .collect();

        let total = runs_in_period.len() as u64;
        let successful = runs_in_period.iter().filter(|r| r.success).count() as u64;
        let failed = total - successful;

        let success_rate = if total > 0 {
            (successful as f32 / total as f32) * 100.0
        } else {
            0.0
        };

        let total_faults: u64 = runs_in_period.iter()
            .map(|r| r.faults.len() as u64)
            .sum();
        let recovered_faults = runs_in_period.iter()
            .map(|r| r.faults.iter().filter(|f| f.ended_at.is_some()).count() as u64)
            .sum();
        let recovery_rate = if total_faults > 0 {
            (recovered_faults as f32 / total_faults as f32) * 100.0
        } else {
            100.0
        };

        let avg_resilience = if !runs_in_period.is_empty() {
            runs_in_period.iter()
                .filter_map(|r| r.results.as_ref())
                .map(|res| res.resilience_score as f32)
                .sum::<f32>() / runs_in_period.len() as f32
        } else {
            0.0
        };

        let avg_availability = if !runs_in_period.is_empty() {
            runs_in_period.iter()
                .filter_map(|r| r.results.as_ref())
                .map(|res| res.availability_percent)
                .sum::<f32>() / runs_in_period.len() as f32
        } else {
            100.0
        };

        let avg_error_rate = if !runs_in_period.is_empty() {
            runs_in_period.iter()
                .filter_map(|r| r.results.as_ref())
                .map(|res| res.error_rate_percent)
                .sum::<f32>() / runs_in_period.len() as f32
        } else {
            0.0
        };

        // Count experiment types
        let mut experiments_by_type: HashMap<String, u64> = HashMap::new();
        for run in &runs_in_period {
            *experiments_by_type.entry(run.name.clone()).or_insert(0) += 1;
        }

        // Count fault types
        let mut faults_by_type: HashMap<String, u64> = HashMap::new();
        for run in &runs_in_period {
            for fault in &run.faults {
                *faults_by_type.entry(fault.fault_type.clone()).or_insert(0) += 1;
            }
        }

        ChaosMetrics {
            total_experiments: total,
            successful_experiments: successful,
            failed_experiments: failed,
            success_rate,
            total_faults_injected: total_faults,
            total_faults_recovered: recovered_faults,
            recovery_rate,
            avg_resilience_score: avg_resilience,
            avg_availability,
            avg_error_rate,
            experiments_by_type,
            faults_by_type,
            period_start: start,
            period_end: end,
        }
    }

    /// Get summary for a specific experiment type
    pub fn get_summary(&self, experiment_name: &str) -> ExperimentSummary {
        let runs: Vec<&ExperimentRun> = self.runs.iter()
            .filter(|r| r.name == experiment_name)
            .collect();

        let total = runs.len() as u64;
        let success = runs.iter().filter(|r| r.success).count() as u64;
        let failure = total - success;

        let avg_duration = if !runs.is_empty() {
            runs.iter()
                .filter_map(|r| r.ended_at.map(|e| (e - r.started_at).num_seconds() as f64))
                .sum::<f64>() / runs.len() as f64
        } else {
            0.0
        };

        let avg_resilience = if !runs.is_empty() {
            runs.iter()
                .filter_map(|r| r.results.as_ref())
                .map(|res| res.resilience_score as f32)
                .sum::<f32>() / runs.len() as f32
        } else {
            0.0
        };

        // Analyze failures
        let mut failure_counts: HashMap<String, u64> = HashMap::new();
        for run in runs.iter().filter(|r| !r.success) {
            for fault in &run.faults {
                if !fault.success {
                    *failure_counts.entry(fault.fault_type.clone()).or_insert(0) += 1;
                }
            }
        }

        let total_failures: u64 = failure_counts.values().sum();
        let most_common_failures: Vec<FailureInfo> = failure_counts
            .into_iter()
            .map(|(ft, count)| FailureInfo {
                failure_type: ft,
                count,
                percentage: if total_failures > 0 {
                    (count as f32 / total_failures as f32) * 100.0
                } else {
                    0.0
                },
                avg_recovery_time_ms: 0, // Would calculate from actual data
            })
            .collect();

        let improvement_trend = TrendDirection::Stable; // Would analyze historical trend

        ExperimentSummary {
            total_runs: total,
            success_count: success,
            failure_count: failure,
            avg_duration_seconds: avg_duration,
            avg_resilience_score: avg_resilience,
            most_common_failures,
            improvement_trend,
        }
    }

    /// Generate a comprehensive report
    pub fn generate_report(&self, period: ReportPeriod) -> ChaosReport {
        let metrics = self.get_metrics(period.start, period.end);

        // Get summaries for all experiment types
        let mut experiment_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for run in &self.runs {
            if run.started_at >= period.start && run.started_at <= period.end {
                experiment_names.insert(run.name.clone());
            }
        }

        let experiment_summaries: Vec<ExperimentSummary> = experiment_names
            .iter()
            .map(|name| self.get_summary(name))
            .collect();

        // Calculate system resilience score (0-100)
        let system_score = ((metrics.success_rate / 100.0) * 40.0
            + (metrics.avg_resilience_score / 100.0) * 30.0
            + (metrics.recovery_rate / 100.0) * 30.0) as u8;

        // Generate recommendations
        let mut recommendations = Vec::new();
        if metrics.success_rate < 80.0 {
            recommendations.push("Success rate below 80%. Review and improve system resilience.".to_string());
        }
        if metrics.avg_error_rate > 5.0 {
            recommendations.push("Average error rate above 5%. Investigate error sources.".to_string());
        }
        if metrics.recovery_rate < 90.0 {
            recommendations.push("Recovery rate below 90%. Improve recovery mechanisms.".to_string());
        }
        if experiment_summaries.iter().any(|s| matches!(s.improvement_trend, TrendDirection::Degrading)) {
            recommendations.push("Some experiments showing degrading trend. Immediate attention required.".to_string());
        }
        if recommendations.is_empty() {
            recommendations.push("System is performing well. Continue regular chaos testing.".to_string());
        }

        ChaosReport {
            id: format!("report-{}", Utc::now().timestamp()),
            generated_at: Utc::now(),
            period,
            metrics,
            experiment_summaries,
            recommendations,
            system_resilience_score: system_score,
        }
    }

    /// Get recent runs
    pub fn get_recent(&self, limit: usize) -> Vec<ExperimentRun> {
        self.runs.iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}

impl Default for ChaosAnalytics {
    fn default() -> Self {
        Self::new()
    }
}

/// Integration with CI/CD systems
pub struct CiCdIntegration {
    pub enabled: bool,
    pub failure_threshold: f32,
}

impl CiCdIntegration {
    pub fn new() -> Self {
        Self {
            enabled: true,
            failure_threshold: 20.0, // Fail CI if failure rate > 20%
        }
    }

    /// Check if CI should fail based on experiment results
    pub fn should_fail_ci(&self, metrics: &ChaosMetrics) -> bool {
        if !self.enabled {
            return false;
        }

        let failure_rate = 100.0 - metrics.success_rate;
        failure_rate > self.failure_threshold
    }

    /// Generate CI-friendly output
    pub fn generate_ci_output(&self, report: &ChaosReport) -> String {
        let mut output = String::new();
        output.push_str(&format!("=== Chaos Engineering Report ===\n"));
        output.push_str(&format!("Generated: {}\n", report.generated_at));
        output.push_str(&format!("Period: {} to {}\n\n", 
            report.period.start, report.period.end));
        
        output.push_str(&format!("Total Experiments: {}\n", report.metrics.total_experiments));
        output.push_str(&format!("Success Rate: {:.1}%\n", report.metrics.success_rate));
        output.push_str(&format!("System Resilience Score: {}/100\n\n", 
            report.system_resilience_score));

        if !report.recommendations.is_empty() {
            output.push_str("Recommendations:\n");
            for rec in &report.recommendations {
                output.push_str(&format!("  - {}\n", rec));
            }
        }

        output
    }
}

impl Default for CiCdIntegration {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chaos_analytics() {
        let mut analytics = ChaosAnalytics::new();

        let run = ExperimentRun {
            id: "test-1".to_string(),
            name: "network-latency".to_string(),
            namespace: "stellar".to_string(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            phase: "Completed".to_string(),
            success: true,
            results: Some(ExperimentResults {
                success: true,
                duration_seconds: 60,
                faults_injected: 1,
                faults_recovered: 1,
                steady_state_validated: true,
                avg_probe_response_ms: 100.0,
                error_rate_percent: 0.5,
                availability_percent: 99.5,
                resilience_score: 85,
                findings: vec![],
            }),
            faults: vec![],
            probe_results: vec![],
        };

        analytics.record(run);

        let metrics = analytics.get_metrics(
            Utc::now() - ChronoDuration::days(1),
            Utc::now()
        );

        assert_eq!(metrics.total_experiments, 1);
        assert_eq!(metrics.successful_experiments, 1);
    }
}