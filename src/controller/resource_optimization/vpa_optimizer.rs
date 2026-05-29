//! Intelligent vertical pod autoscaling recommendations based on ML forecasts.

use serde::{Deserialize, Serialize};

/// VPA optimization configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaOptimization {
    /// Enable ML-driven VPA recommendations.
    pub enabled: bool,
    /// Safety margin multiplier for CPU recommendations.
    #[serde(default = "default_cpu_margin")]
    pub cpu_margin: f64,
    /// Safety margin multiplier for memory recommendations.
    #[serde(default = "default_memory_margin")]
    pub memory_margin: f64,
    /// Minimum CPU request (e.g. "100m").
    #[serde(default = "default_min_cpu")]
    pub min_cpu: String,
    /// Maximum CPU limit (e.g. "8").
    #[serde(default = "default_max_cpu")]
    pub max_cpu: String,
    /// Minimum memory request (e.g. "256Mi").
    #[serde(default = "default_min_memory")]
    pub min_memory: String,
    /// Maximum memory limit (e.g. "16Gi").
    #[serde(default = "default_max_memory")]
    pub max_memory: String,
}

fn default_cpu_margin() -> f64 {
    1.15
}
fn default_memory_margin() -> f64 {
    1.20
}
fn default_min_cpu() -> String {
    "100m".to_string()
}
fn default_max_cpu() -> String {
    "8".to_string()
}
fn default_min_memory() -> String {
    "256Mi".to_string()
}
fn default_max_memory() -> String {
    "16Gi".to_string()
}

impl Default for VpaOptimization {
    fn default() -> Self {
        Self {
            enabled: false,
            cpu_margin: default_cpu_margin(),
            memory_margin: default_memory_margin(),
            min_cpu: default_min_cpu(),
            max_cpu: default_max_cpu(),
            min_memory: default_min_memory(),
            max_memory: default_max_memory(),
        }
    }
}

/// Resource usage observation for VPA right-sizing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceObservation {
    pub cpu_millicores: f64,
    pub memory_bytes: f64,
    pub forecast_cpu_millicores: f64,
    pub forecast_memory_bytes: f64,
}

/// VPA right-sizing recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpaRecommendation {
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub confidence: f64,
    pub rationale: String,
}

/// Computes VPA recommendations from observed and forecasted usage.
pub struct VpaOptimizer;

impl VpaOptimizer {
    pub fn recommend(config: &VpaOptimization, obs: &ResourceObservation) -> VpaRecommendation {
        let target_cpu = obs.forecast_cpu_millicores * config.cpu_margin;
        let target_mem = obs.forecast_memory_bytes * config.memory_margin;

        let cpu_request = format_cpu_millicores(target_cpu);
        let cpu_limit = format_cpu_millicores(target_cpu * 1.5);
        let memory_request = format_memory_bytes(target_mem);
        let memory_limit = format_memory_bytes(target_mem * 1.25);

        VpaRecommendation {
            cpu_request: clamp_cpu(&cpu_request, &config.min_cpu, &config.max_cpu),
            cpu_limit: clamp_cpu(&cpu_limit, &config.min_cpu, &config.max_cpu),
            memory_request: clamp_memory(&memory_request, &config.min_memory, &config.max_memory),
            memory_limit: clamp_memory(&memory_limit, &config.min_memory, &config.max_memory),
            confidence: 85.0,
            rationale: format!(
                "Forecast CPU {:.0}m, memory {:.1}Gi with {:.0}%/{:.0}% margins",
                obs.forecast_cpu_millicores,
                obs.forecast_memory_bytes / (1024.0 * 1024.0 * 1024.0),
                (config.cpu_margin - 1.0) * 100.0,
                (config.memory_margin - 1.0) * 100.0,
            ),
        }
    }
}

fn format_cpu_millicores(m: f64) -> String {
    if m >= 1000.0 {
        format!("{}", (m / 1000.0).ceil() as u32)
    } else {
        format!("{}m", m.ceil() as u32)
    }
}

fn format_memory_bytes(b: f64) -> String {
    let gib = b / (1024.0 * 1024.0 * 1024.0);
    if gib >= 1.0 {
        format!("{}Gi", gib.ceil() as u32)
    } else {
        let mib = b / (1024.0 * 1024.0);
        format!("{}Mi", mib.ceil() as u32)
    }
}

fn parse_cpu(s: &str) -> f64 {
    if s.ends_with('m') {
        s.trim_end_matches('m').parse().unwrap_or(0.0)
    } else {
        s.parse::<f64>().unwrap_or(0.0) * 1000.0
    }
}

fn parse_memory(s: &str) -> f64 {
    if s.ends_with("Gi") {
        s.trim_end_matches("Gi").parse::<f64>().unwrap_or(0.0) * 1024.0 * 1024.0 * 1024.0
    } else if s.ends_with("Mi") {
        s.trim_end_matches("Mi").parse::<f64>().unwrap_or(0.0) * 1024.0 * 1024.0
    } else {
        0.0
    }
}

fn clamp_cpu(value: &str, min: &str, max: &str) -> String {
    let v = parse_cpu(value);
    let lo = parse_cpu(min);
    let hi = parse_cpu(max);
    format_cpu_millicores(v.clamp(lo, hi))
}

fn clamp_memory(value: &str, min: &str, max: &str) -> String {
    let v = parse_memory(value);
    let lo = parse_memory(min);
    let hi = parse_memory(max);
    format_memory_bytes(v.clamp(lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_scaled_resources() {
        let config = VpaOptimization::default();
        let obs = ResourceObservation {
            cpu_millicores: 500.0,
            memory_bytes: 512.0 * 1024.0 * 1024.0,
            forecast_cpu_millicores: 800.0,
            forecast_memory_bytes: 1024.0 * 1024.0 * 1024.0,
        };
        let rec = VpaOptimizer::recommend(&config, &obs);
        assert!(rec.cpu_request.contains('m') || rec.cpu_request.parse::<u32>().is_ok());
        assert!(rec.memory_request.ends_with("Gi") || rec.memory_request.ends_with("Mi"));
        assert!(rec.confidence > 0.0);
    }
}
