//! Cost forecasting with trend analysis and projection

use serde::{Deserialize, Serialize};

/// Cost forecast result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostForecast {
    pub resource_id: String,
    pub forecast_30d_usd: f64,
    pub forecast_90d_usd: f64,
    pub trend: Trend,
    pub confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Trend {
    Increasing,
    Decreasing,
    Stable,
}

pub struct CostForecaster;

impl CostForecaster {
    /// Simple linear regression forecast over historical daily costs
    pub fn forecast(resource_id: &str, daily_costs: &[f64]) -> Option<CostForecast> {
        let n = daily_costs.len();
        if n < 3 {
            return None;
        }

        // Least-squares linear regression: y = a + b*x
        let n_f = n as f64;
        let x_mean = (n_f - 1.0) / 2.0;
        let y_mean = daily_costs.iter().sum::<f64>() / n_f;

        let mut num = 0.0_f64;
        let mut den = 0.0_f64;
        for (i, &y) in daily_costs.iter().enumerate() {
            let x = i as f64;
            num += (x - x_mean) * (y - y_mean);
            den += (x - x_mean).powi(2);
        }

        let slope = if den != 0.0 { num / den } else { 0.0 };
        let intercept = y_mean - slope * x_mean;

        let predict_day = |d: f64| (intercept + slope * d).max(0.0);
        let forecast_30d = (0..30).map(|d| predict_day(n as f64 + d as f64)).sum();
        let forecast_90d = (0..90).map(|d| predict_day(n as f64 + d as f64)).sum();

        let trend = if slope > 0.01 * y_mean {
            Trend::Increasing
        } else if slope < -0.01 * y_mean {
            Trend::Decreasing
        } else {
            Trend::Stable
        };

        // Confidence: higher with more data points
        let confidence = (n as f64 / 30.0).min(1.0);

        Some(CostForecast {
            resource_id: resource_id.into(),
            forecast_30d_usd: forecast_30d,
            forecast_90d_usd: forecast_90d,
            trend,
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stable_forecast() {
        let daily = vec![100.0_f64; 14];
        let fc = CostForecaster::forecast("r1", &daily).unwrap();
        assert_eq!(fc.trend, Trend::Stable);
        assert!((fc.forecast_30d_usd - 3000.0).abs() < 10.0);
    }

    #[test]
    fn test_increasing_trend() {
        let daily: Vec<f64> = (0..14).map(|i| 100.0 + i as f64 * 10.0).collect();
        let fc = CostForecaster::forecast("r1", &daily).unwrap();
        assert_eq!(fc.trend, Trend::Increasing);
    }

    #[test]
    fn test_insufficient_data_returns_none() {
        assert!(CostForecaster::forecast("r1", &[100.0, 100.0]).is_none());
    }
}
