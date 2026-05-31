//! Compliance dashboard with real-time status.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::policy_engine::PolicyViolation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceDashboard {
    pub overall_score: f64, // 0-100
    pub framework_scores: HashMap<String, f64>,
    pub active_violations: usize,
    pub critical_violations: usize,
    pub last_scan_at: DateTime<Utc>,
    pub trend: Vec<DailyScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyScore {
    pub date: String,
    pub score: f64,
    pub violations: usize,
}

pub struct DashboardService {
    history: Vec<(DateTime<Utc>, Vec<PolicyViolation>)>,
}

impl DashboardService {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
        }
    }

    pub fn record_scan(&mut self, violations: Vec<PolicyViolation>) {
        self.history.push((Utc::now(), violations));
        // Keep last 30 days
        if self.history.len() > 30 {
            self.history.remove(0);
        }
    }

    pub fn compute_dashboard(&self, violations: &[PolicyViolation]) -> ComplianceDashboard {
        let active = violations.len();
        let critical = violations
            .iter()
            .filter(|v| {
                matches!(
                    v.severity,
                    super::policy_engine::ViolationSeverity::Critical
                )
            })
            .count();

        // Score: 100 - (weighted violations)
        let weighted: f64 = violations
            .iter()
            .map(|v| match v.severity {
                super::policy_engine::ViolationSeverity::Critical => 20.0,
                super::policy_engine::ViolationSeverity::High => 10.0,
                super::policy_engine::ViolationSeverity::Medium => 5.0,
                super::policy_engine::ViolationSeverity::Low => 1.0,
            })
            .sum();
        let overall_score = (100.0 - weighted).max(0.0);

        // Per-framework scores
        let mut framework_violations: HashMap<String, Vec<&PolicyViolation>> = HashMap::new();
        for v in violations {
            // Extract framework from policy_id prefix
            let fw = if v.policy_id.starts_with("soc2") {
                "SOC2"
            } else if v.policy_id.starts_with("iso27001") {
                "ISO27001"
            } else if v.policy_id.starts_with("pci") {
                "PCI-DSS"
            } else {
                "Other"
            };
            framework_violations
                .entry(fw.to_string())
                .or_default()
                .push(v);
        }

        let framework_scores: HashMap<String, f64> = ["SOC2", "ISO27001", "PCI-DSS"]
            .iter()
            .map(|fw| {
                let fw_violations = framework_violations.get(*fw).map(|v| v.len()).unwrap_or(0);
                let score = (100.0 - fw_violations as f64 * 10.0).max(0.0);
                (fw.to_string(), score)
            })
            .collect();

        let trend = self.get_trend(7);

        ComplianceDashboard {
            overall_score,
            framework_scores,
            active_violations: active,
            critical_violations: critical,
            last_scan_at: Utc::now(),
            trend,
        }
    }

    pub fn get_trend(&self, days: usize) -> Vec<DailyScore> {
        self.history
            .iter()
            .rev()
            .take(days)
            .map(|(ts, violations)| {
                let weighted: f64 = violations
                    .iter()
                    .map(|v| match v.severity {
                        super::policy_engine::ViolationSeverity::Critical => 20.0,
                        super::policy_engine::ViolationSeverity::High => 10.0,
                        super::policy_engine::ViolationSeverity::Medium => 5.0,
                        super::policy_engine::ViolationSeverity::Low => 1.0,
                    })
                    .sum();
                DailyScore {
                    date: ts.format("%Y-%m-%d").to_string(),
                    score: (100.0 - weighted).max(0.0),
                    violations: violations.len(),
                }
            })
            .collect()
    }
}

impl Default for DashboardService {
    fn default() -> Self {
        Self::new()
    }
}
