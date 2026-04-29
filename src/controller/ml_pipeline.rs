use crate::controller::audit_log::AuditEntry;
use crate::error::Result;
use std::collections::HashMap;

/// Feature vector for ML anomaly detection
#[derive(Debug, Clone, Default)]
pub struct FeatureVector {
    pub action_frequency: HashMap<String, f64>,
    pub error_rate: f64,
    pub latency_avg: f64,
    pub unique_actors: usize,
}

/// Trait for anomaly detection models
pub trait AnomalyModel: Send + Sync {
    /// Predict if the current feature vector is anomalous
    fn predict(&self, features: &FeatureVector) -> Prediction;
    /// Train the model with a batch of features
    fn train(&self, batch: &[FeatureVector]) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct Prediction {
    pub is_anomaly: bool,
    pub confidence: f64,
    pub explanation: String,
}

/// Extract features from a window of audit entries
pub fn extract_features(entries: &[AuditEntry]) -> FeatureVector {
    let mut features = FeatureVector::default();
    if entries.is_empty() {
        return features;
    }

    let mut actors = std::collections::HashSet::new();
    let mut errors = 0;

    for entry in entries {
        *features.action_frequency.entry(format!("{:?}", entry.action)).or_insert(0.0) += 1.0;
        if !entry.success {
            errors += 1;
        }
        actors.insert(entry.actor.clone());
    }

    let total = entries.len() as f64;
    for val in features.action_frequency.values_mut() {
        *val /= total;
    }

    features.error_rate = errors as f64 / total;
    features.unique_actors = actors.len();

    features
}

/// Simple EWMA-based anomaly detector (placeholder for more complex ML)
pub struct EwmaModel {
    #[allow(dead_code)]
    alpha: f64,
    threshold: f64,
}

impl EwmaModel {
    pub fn new(alpha: f64, threshold: f64) -> Self {
        Self { alpha, threshold }
    }
}

impl AnomalyModel for EwmaModel {
    fn predict(&self, features: &FeatureVector) -> Prediction {
        // Simulated prediction logic
        Prediction {
            is_anomaly: features.error_rate > self.threshold,
            confidence: 0.85,
            explanation: format!("Error rate {} exceeds threshold {}", features.error_rate, self.threshold),
        }
    }

    fn train(&self, _batch: &[FeatureVector]) -> Result<()> {
        Ok(())
    }
}
