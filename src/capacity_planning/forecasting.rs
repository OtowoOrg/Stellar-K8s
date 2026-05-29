//! Growth Forecasting and Trend Detection
//!
//! Implements multiple forecasting models to predict future resource needs.

use crate::capacity_planning::ResourceUsage;
use chrono::{DateTime, Duration, Utc};

pub enum ForecastModel {
    Linear,
    Exponential,
    HoltWinters,
}

pub struct Forecaster {
    pub model: ForecastModel,
}

impl Forecaster {
    pub fn new(model: ForecastModel) -> Self {
        Self { model }
    }

    /// Predicts future resource usage based on historical data
    pub fn forecast(&self, history: &[ResourceUsage], horizon_days: u32) -> Vec<(DateTime<Utc>, f64)> {
        if history.is_empty() {
            return Vec::new();
        }

        match self.model {
            ForecastModel::Linear => self.linear_forecast(history, horizon_days),
            ForecastModel::Exponential => self.exponential_forecast(history, horizon_days),
            ForecastModel::HoltWinters => self.holt_winters_forecast(history, horizon_days),
        }
    }

    fn linear_forecast(&self, history: &[ResourceUsage], horizon_days: u32) -> Vec<(DateTime<Utc>, f64)> {
        // Simple linear regression: y = mx + b
        let n = history.len() as f64;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_xy = 0.0;
        let mut sum_xx = 0.0;

        let start_time = history[0].timestamp.timestamp() as f64;

        for (i, entry) in history.iter().enumerate() {
            let x = (entry.timestamp.timestamp() as f64 - start_time) / 86400.0; // days
            let y = entry.cpu_cores; // Example: forecasting CPU
            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_xx += x * x;
        }

        let m = (n * sum_xy - sum_x * sum_y) / (n * sum_xx - sum_x * sum_x);
        let b = (sum_y - m * sum_x) / n;

        let last_time = history.last().unwrap().timestamp;
        let mut forecast = Vec::new();

        for day in 1..=horizon_days {
            let x = ((last_time + Duration::days(day as i64)).timestamp() as f64 - start_time) / 86400.0;
            let y = m * x + b;
            forecast.push((last_time + Duration::days(day as i64), y));
        }

        forecast
    }

    fn exponential_forecast(&self, history: &[ResourceUsage], horizon_days: u32) -> Vec<(DateTime<Utc>, f64)> {
        // y = a * e^(bx) -> ln(y) = ln(a) + bx
        // We reuse linear regression on ln(y)
        let n = history.len() as f64;
        let mut sum_x = 0.0;
        let mut sum_y_ln = 0.0;
        let mut sum_xy_ln = 0.0;
        let mut sum_xx = 0.0;

        let start_time = history[0].timestamp.timestamp() as f64;

        for entry in history {
            let x = (entry.timestamp.timestamp() as f64 - start_time) / 86400.0;
            let y = entry.cpu_cores.max(0.001); // Avoid ln(0)
            let y_ln = y.ln();
            sum_x += x;
            sum_y_ln += y_ln;
            sum_xy_ln += x * y_ln;
            sum_xx += x * x;
        }

        let b = (n * sum_xy_ln - sum_x * sum_y_ln) / (n * sum_xx - sum_x * sum_x);
        let a_ln = (sum_y_ln - b * sum_x) / n;
        let a = a_ln.exp();

        let last_time = history.last().unwrap().timestamp;
        let mut forecast = Vec::new();

        for day in 1..=horizon_days {
            let x = ((last_time + Duration::days(day as i64)).timestamp() as f64 - start_time) / 86400.0;
            let y = a * (b * x).exp();
            forecast.push((last_time + Duration::days(day as i64), y));
        }

        forecast
    }

    fn holt_winters_forecast(&self, _history: &[ResourceUsage], _horizon_days: u32) -> Vec<(DateTime<Utc>, f64)> {
        // Placeholder for more complex seasonal forecasting
        // In a real implementation, this would use double/triple exponential smoothing
        Vec::new()
    }
}
