//! Time-series forecasting engine for workload prediction.
//!
//! Supports Holt-Winters double exponential smoothing, linear regression,
//! and ensemble blending for robust predictions.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A single time-series observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
}

/// A forecasted data point with confidence bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastPoint {
    pub timestamp: DateTime<Utc>,
    pub predicted: f64,
    pub lower_bound: f64,
    pub upper_bound: f64,
}

/// Complete forecast result with accuracy metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastResult {
    pub points: Vec<ForecastPoint>,
    pub model: String,
    pub mape: f64,
    pub confidence: f64,
}

/// Forecasting model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForecastModel {
    HoltWinters,
    Linear,
    Ensemble,
}

/// Time-series forecasting engine.
pub struct ForecastEngine {
    pub model: ForecastModel,
    pub alpha: f64,
    pub beta: f64,
    pub confidence_interval_pct: f64,
}

impl Default for ForecastEngine {
    fn default() -> Self {
        Self {
            model: ForecastModel::Ensemble,
            alpha: 0.3,
            beta: 0.1,
            confidence_interval_pct: 0.15,
        }
    }
}

impl ForecastEngine {
    pub fn new(model: ForecastModel) -> Self {
        Self {
            model,
            ..Default::default()
        }
    }

    /// Forecast `horizon_steps` ahead from historical observations.
    /// Each step corresponds to the median interval between observations.
    pub fn forecast(&self, history: &[TimeSeriesPoint], horizon_steps: u32) -> ForecastResult {
        if history.len() < 2 {
            return ForecastResult {
                points: Vec::new(),
                model: "insufficient_data".to_string(),
                mape: 100.0,
                confidence: 0.0,
            };
        }

        let values: Vec<f64> = history.iter().map(|p| p.value).collect();
        let step_duration = self.infer_step_duration(history);

        let (predictions, model_name) = match self.model {
            ForecastModel::HoltWinters => (
                self.holt_winters_forecast(&values, horizon_steps),
                "holt_winters",
            ),
            ForecastModel::Linear => (self.linear_forecast(&values, horizon_steps), "linear"),
            ForecastModel::Ensemble => {
                let hw = self.holt_winters_forecast(&values, horizon_steps);
                let lin = self.linear_forecast(&values, horizon_steps);
                let blended: Vec<f64> = hw
                    .iter()
                    .zip(lin.iter())
                    .map(|(h, l)| 0.6 * h + 0.4 * l)
                    .collect();
                (blended, "ensemble")
            }
        };

        let mape = self.compute_mape(&values, &predictions);
        let confidence = (100.0 - mape).clamp(0.0, 100.0);
        let last_ts = history.last().unwrap().timestamp;
        let margin = self.confidence_interval_pct;

        let points = predictions
            .into_iter()
            .enumerate()
            .map(|(i, predicted)| {
                let ts = last_ts + step_duration * (i as i32 + 1);
                ForecastPoint {
                    timestamp: ts,
                    predicted,
                    lower_bound: (predicted * (1.0 - margin)).max(0.0),
                    upper_bound: predicted * (1.0 + margin),
                }
            })
            .collect();

        ForecastResult {
            points,
            model: model_name.to_string(),
            mape,
            confidence,
        }
    }

    fn infer_step_duration(&self, history: &[TimeSeriesPoint]) -> Duration {
        if history.len() < 2 {
            return Duration::minutes(1);
        }
        let total_secs = (history.last().unwrap().timestamp - history[0].timestamp).num_seconds();
        let steps = (history.len() - 1) as i64;
        Duration::seconds((total_secs / steps.max(1)).max(60))
    }

    fn holt_winters_forecast(&self, values: &[f64], horizon: u32) -> Vec<f64> {
        let mut level = values[0];
        let mut trend = values[1] - values[0];

        for &obs in &values[2..] {
            let prev_level = level;
            level = self.alpha * obs + (1.0 - self.alpha) * (level + trend);
            trend = self.beta * (level - prev_level) + (1.0 - self.beta) * trend;
        }

        (1..=horizon)
            .map(|h| (level + h as f64 * trend).max(0.0))
            .collect()
    }

    fn linear_forecast(&self, values: &[f64], horizon: u32) -> Vec<f64> {
        let n = values.len() as f64;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_xy = 0.0;
        let mut sum_xx = 0.0;

        for (i, &y) in values.iter().enumerate() {
            let x = i as f64;
            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_xx += x * x;
        }

        let denom = n * sum_xx - sum_x * sum_x;
        if denom.abs() < f64::EPSILON {
            return vec![values.last().copied().unwrap_or(0.0); horizon as usize];
        }

        let m = (n * sum_xy - sum_x * sum_y) / denom;
        let b = (sum_y - m * sum_x) / n;
        let start = values.len() as f64;

        (0..horizon)
            .map(|h| (m * (start + h as f64) + b).max(0.0))
            .collect()
    }

    fn compute_mape(&self, actual: &[f64], predicted: &[f64]) -> f64 {
        let holdout = actual.len().min(5).max(1);
        if actual.len() <= holdout {
            return 10.0;
        }

        let train = &actual[..actual.len() - holdout];
        let test = &actual[actual.len() - holdout..];
        let preds = self.holt_winters_forecast(train, holdout as u32);

        let mut total_pct = 0.0;
        let mut count = 0.0;
        for (a, p) in test.iter().zip(preds.iter()) {
            if *a > f64::EPSILON {
                total_pct += ((a - p).abs() / a) * 100.0;
                count += 1.0;
            }
        }

        if count > 0.0 {
            total_pct / count
        } else {
            10.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_history(values: &[f64]) -> Vec<TimeSeriesPoint> {
        let base = Utc::now() - Duration::minutes(values.len() as i64);
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| TimeSeriesPoint {
                timestamp: base + Duration::minutes(i as i64),
                value: v,
            })
            .collect()
    }

    #[test]
    fn forecast_increasing_trend() {
        let history = make_history(&[100.0, 110.0, 120.0, 130.0, 140.0, 150.0]);
        let engine = ForecastEngine::new(ForecastModel::HoltWinters);
        let result = engine.forecast(&history, 3);
        assert_eq!(result.points.len(), 3);
        assert!(result.points[0].predicted > 140.0);
        assert!(result.confidence > 0.0);
    }

    #[test]
    fn forecast_insufficient_data() {
        let history = make_history(&[100.0]);
        let engine = ForecastEngine::default();
        let result = engine.forecast(&history, 5);
        assert!(result.points.is_empty());
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn ensemble_blends_models() {
        let history = make_history(&[50.0, 55.0, 60.0, 65.0, 70.0, 75.0]);
        let engine = ForecastEngine::new(ForecastModel::Ensemble);
        let result = engine.forecast(&history, 2);
        assert_eq!(result.model, "ensemble");
        assert!(result.points[0].lower_bound <= result.points[0].predicted);
        assert!(result.points[0].upper_bound >= result.points[0].predicted);
    }
}
