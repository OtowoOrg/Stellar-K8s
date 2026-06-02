//! Cost anomaly detection with configurable alerting thresholds

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::warn;

use super::model::CostRecord;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostAnomaly {
    pub resource_id: String,
    pub expected_cost: f64,
    pub actual_cost: f64,
    pub deviation_pct: f64,
    pub severity: AnomalySeverity,
    pub detected_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnomalySeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Rolling-window anomaly detector using z-score
pub struct AnomalyDetector {
    /// Cost history per resource_id
    history: std::collections::HashMap<String, VecDeque<f64>>,
    window_size: usize,
    /// Alert when cost deviates > threshold * stddev
    z_threshold: f64,
    /// Alert thresholds by pct deviation
    low_pct: f64,
    medium_pct: f64,
    high_pct: f64,
}

impl AnomalyDetector {
    pub fn new(window_size: usize, z_threshold: f64) -> Self {
        Self {
            history: std::collections::HashMap::new(),
            window_size,
            z_threshold,
            low_pct: 20.0,
            medium_pct: 50.0,
            high_pct: 100.0,
        }
    }

    pub fn observe(&mut self, record: &CostRecord) -> Option<CostAnomaly> {
        let h = self.history.entry(record.resource_id.clone()).or_default();
        if h.len() >= self.window_size {
            h.pop_front();
        }

        let anomaly = if h.len() >= 3 {
            let mean = h.iter().sum::<f64>() / h.len() as f64;
            let variance = h.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / h.len() as f64;
            let stddev = variance.sqrt();

            if stddev > 0.0 {
                let z = (record.cost_usd - mean) / stddev;
                if z.abs() > self.z_threshold {
                    let deviation_pct = ((record.cost_usd - mean) / mean * 100.0).abs();
                    let severity = if deviation_pct >= self.high_pct {
                        AnomalySeverity::High
                    } else if deviation_pct >= self.medium_pct {
                        AnomalySeverity::Medium
                    } else {
                        AnomalySeverity::Low
                    };
                    warn!(
                        resource = %record.resource_id,
                        expected = mean,
                        actual = record.cost_usd,
                        z_score = z,
                        "Cost anomaly detected"
                    );
                    Some(CostAnomaly {
                        resource_id: record.resource_id.clone(),
                        expected_cost: mean,
                        actual_cost: record.cost_usd,
                        deviation_pct,
                        severity,
                        detected_at: Utc::now(),
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        h.push_back(record.cost_usd);
        anomaly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use super::super::model::{CloudProvider, ResourceType};

    fn record(id: &str, cost: f64) -> CostRecord {
        CostRecord {
            id: id.into(),
            provider: CloudProvider::Aws,
            resource_type: ResourceType::ComputeInstance,
            resource_id: id.into(),
            namespace: "ns".into(),
            team: "team".into(),
            region: "us-east-1".into(),
            instance_type: "m5.large".into(),
            cost_usd: cost,
            usage_hours: 24.0,
            period_start: Utc::now(),
            period_end: Utc::now(),
            tags: HashMap::new(),
            is_spot: false,
            is_reserved: false,
        }
    }

    #[test]
    fn test_normal_costs_no_anomaly() {
        let mut det = AnomalyDetector::new(10, 2.0);
        for _ in 0..8 { det.observe(&record("r1", 100.0)); }
        let result = det.observe(&record("r1", 105.0));
        assert!(result.is_none());
    }

    #[test]
    fn test_spike_detected() {
        let mut det = AnomalyDetector::new(10, 2.0);
        for _ in 0..8 { det.observe(&record("r1", 100.0)); }
        let result = det.observe(&record("r1", 500.0));
        assert!(result.is_some());
        assert!(result.unwrap().severity == AnomalySeverity::High);
    }
}
