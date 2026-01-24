//! Differential Privacy utilities for telemetry
//!
//! Provides mechanisms to add Laplace noise to reported counts and metrics
//! to protect individual node privacy while maintaining aggregate utility.

use rand::distributions::Distribution;
use rand_distr::Exp;

/// Differential privacy configuration
#[derive(Debug, Clone)]
pub struct PrivacyConfig {
    /// Epsilon parameter for differential privacy (lower means more privacy)
    pub epsilon: f64,
    /// Sensitivity of the query (maximum change one individual can cause)
    pub sensitivity: f64,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            epsilon: 0.1,
            sensitivity: 1.0,
        }
    }
}

/// Apply Laplace noise to a count using the difference of two Exponential distributions
pub fn add_laplace_noise(value: f64, config: &PrivacyConfig) -> f64 {
    let b = config.sensitivity / config.epsilon;
    let exp = Exp::new(1.0 / b).expect("Invalid Exponential parameters");
    let mut rng = rand::thread_rng();
    // Laplace(0, b) is equivalent to Exp(1/b) - Exp(1/b)
    let noise = exp.sample(&mut rng) - exp.sample(&mut rng);
    value + noise
}

/// A wrapper for metrics that applies differential privacy
pub struct PrivancyAwareMetric {
    config: PrivacyConfig,
}

impl PrivancyAwareMetric {
    pub fn new(config: PrivacyConfig) -> Self {
        Self { config }
    }

    /// Scrub sensitive labels from a metric
    pub fn scrub_labels(labels: &mut std::collections::HashMap<String, String>) {
        let sensitive_keys = ["ip", "host.ip", "k8s.cluster.name", "cluster_name", "pod_name"];
        for key in sensitive_keys {
            if labels.contains_key(key) {
                labels.insert(key.to_string(), "REDACTED".to_string());
            }
        }
    }

    /// Protect a count value
    pub fn protect_count(&self, value: u64) -> u64 {
        let noisy_value = add_laplace_noise(value as f64, &self.config);
        if noisy_value < 0.0 {
            0
        } else {
            noisy_value.round() as u64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_laplace_noise_adds_noise() {
        let config = PrivacyConfig { epsilon: 0.1, sensitivity: 1.0 };
        let original = 100.0;
        let mut different = false;
        
        // With epsilon 0.1, noise should be non-zero most of the time
        for _ in 0..10 {
            let noisy = add_laplace_noise(original, &config);
            if (noisy - original).abs() > 0.0001 {
                different = true;
                break;
            }
        }
        assert!(different, "Laplace noise should modify the value");
    }

    #[test]
    fn test_protect_count_is_non_negative() {
        let config = PrivacyConfig { epsilon: 10.0, sensitivity: 1.0 };
        let engine = PrivancyAwareMetric::new(config);
        
        for _ in 0..100 {
            let protected = engine.protect_count(0);
            assert!(protected >= 0, "Protected count must be non-negative");
        }
    }

    #[test]
    fn test_scrub_labels() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("ip".to_string(), "1.2.3.4".to_string());
        labels.insert("cluster_name".to_string(), "prod-cluster".to_string());
        labels.insert("service".to_string(), "stellar".to_string());
        
        PrivancyAwareMetric::scrub_labels(&mut labels);
        
        assert_eq!(labels.get("ip").unwrap(), "REDACTED");
        assert_eq!(labels.get("cluster_name").unwrap(), "REDACTED");
        assert_eq!(labels.get("service").unwrap(), "stellar");
    }
}
