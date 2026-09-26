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
//! Capacity Forecasting Engine with Quarterly Scaling Recommendations
//!
//! Forecasts cluster capacity exhaustion 90 days out from utilisation trends
//! and emits scaling recommendations ranked by time-to-exhaustion.
//!
//! # Design
//!
//! - CPU, memory, storage and object counts are separate series, each
//!   resampled to daily peaks and forecast independently.
//! - Every series is first fitted with a robust linear model (Theil–Sen
//!   slope, MAD-based residual scale). A seasonal model (robust trend plus an
//!   additive weekly profile) is used only when its rolling-origin backtest
//!   MAPE beats the linear model by a configurable margin.
//! - Every forecast carries a confidence interval that widens with distance
//!   from the training data; time-to-exhaustion is reported for the upper,
//!   point and lower bands.
//! - Each cycle backtests the model version against the supplied history and
//!   against historical capacity incidents, and the result is published with
//!   the recommendations as a [`CapacityRecommendationReport`] CR.
//!
//! Capacity is taken per sample, so cluster-autoscaler history is reflected
//! simply by feeding the capacity that was in effect at each timestamp.
//!
//! ## Acceptance Criteria (from #1493)
//! - Forecast MAPE below 15% at 90-day horizon (`BacktestReport::mape_target_met`)
//! - Every capacity exhaustion predicted >= 14 days ahead
//!   (`BacktestReport::lead_time_target_met`)
//! - Backtest published with each recommendation cycle
//! - Recommendations exported as CR for automation ([`publish_report`])

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use kube::api::{Api, Patch, PatchParams};
use thiserror::Error;
use tracing::{debug, warn};

use crate::crd::capacity_forecast::{
    BacktestReport, CapacityDimension, CapacityForecastSummary, CapacityRecommendationReport,
    CapacityRecommendationReportSpec, ForecastInterval, ForecastModelKind, IncidentBacktest,
    RecommendationPriority, ScalingRecommendation, SeriesBacktest, TimeToExhaustion,
};

/// Version stamped on every forecast, recommendation and backtest.
pub const MODEL_VERSION: &str = "capacity-forecast/1.0.0";

const FIELD_MANAGER: &str = "stellar-capacity-forecast";
const SECONDS_PER_DAY: i64 = 86_400;
/// Scale factor turning a median absolute deviation into a normal sigma.
const MAD_TO_SIGMA: f64 = 1.4826;

/// A single utilisation observation from the metrics store.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UtilizationPoint {
    pub timestamp: DateTime<Utc>,
    pub used: f64,
    /// Capacity in effect at `timestamp` (from cluster-autoscaler history).
    pub capacity: f64,
}

/// Utilisation history of one capacity dimension of one cluster.
#[derive(Debug, Clone)]
pub struct UtilizationSeries {
    pub cluster: String,
    pub dimension: CapacityDimension,
    /// Object kind for [`CapacityDimension::ObjectCount`] series.
    pub object_kind: Option<String>,
    pub points: Vec<UtilizationPoint>,
}

/// A historical capacity exhaustion incident used for backtesting.
#[derive(Debug, Clone)]
pub struct CapacityIncident {
    pub cluster: String,
    pub dimension: CapacityDimension,
    pub object_kind: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForecastConfig {
    pub horizon_days: u32,
    /// Two-sided confidence level of the forecast intervals.
    pub confidence_level: f64,
    /// Minimum daily samples required to fit a series.
    pub min_history_days: u32,
    /// Trailing window of daily samples used to fit each model.
    pub lookback_days: u32,
    /// Spacing between rolling backtest origins.
    pub backtest_step_days: u32,
    /// Required warning lead time before exhaustion.
    pub min_lead_time_days: u32,
    pub seasonal_period_days: u32,
    /// Relative MAPE improvement the seasonal model must show to be selected.
    pub seasonal_min_improvement: f64,
    /// Fraction of capacity at which the dimension counts as exhausted.
    pub exhaustion_threshold: f64,
    /// Utilisation the recommended capacity is sized for at the upper band.
    pub target_utilization: f64,
    pub mape_target_pct: f64,
}

impl Default for ForecastConfig {
    fn default() -> Self {
        Self {
            horizon_days: 90,
            confidence_level: 0.9,
            min_history_days: 30,
            lookback_days: 365,
            backtest_step_days: 7,
            min_lead_time_days: 14,
            seasonal_period_days: 7,
            seasonal_min_improvement: 0.1,
            exhaustion_threshold: 1.0,
            target_utilization: 0.8,
            mape_target_pct: 15.0,
        }
    }
}

impl ForecastConfig {
    pub fn validate(&self) -> Result<(), ForecastError> {
        let invalid = |msg: &str| Err(ForecastError::InvalidConfig(msg.to_string()));
        if self.horizon_days == 0 {
            return invalid("horizonDays must be > 0");
        }
        if !(self.confidence_level > 0.0 && self.confidence_level < 1.0) {
            return invalid("confidenceLevel must be in (0, 1)");
        }
        if self.min_history_days < 2 {
            return invalid("minHistoryDays must be >= 2");
        }
        if self.lookback_days < self.min_history_days {
            return invalid("lookbackDays must be >= minHistoryDays");
        }
        if self.backtest_step_days == 0 {
            return invalid("backtestStepDays must be > 0");
        }
        if self.seasonal_period_days < 2 {
            return invalid("seasonalPeriodDays must be >= 2");
        }
        if !(0.0..1.0).contains(&self.seasonal_min_improvement) {
            return invalid("seasonalMinImprovement must be in [0, 1)");
        }
        if !(self.exhaustion_threshold > 0.0 && self.exhaustion_threshold <= 1.0) {
            return invalid("exhaustionThreshold must be in (0, 1]");
        }
        if !(self.target_utilization > 0.0 && self.target_utilization <= 1.0) {
            return invalid("targetUtilization must be in (0, 1]");
        }
        if self.mape_target_pct <= 0.0 {
            return invalid("mapeTargetPct must be > 0");
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ForecastError {
    #[error("invalid forecast configuration: {0}")]
    InvalidConfig(String),
}

/// Runs recommendation cycles over utilisation history.
#[derive(Debug, Clone)]
pub struct CapacityForecastEngine {
    config: ForecastConfig,
    z: f64,
}

impl CapacityForecastEngine {
    pub fn new(config: ForecastConfig) -> Result<Self, ForecastError> {
        config.validate()?;
        let z = normal_quantile(0.5 + config.confidence_level / 2.0);
        Ok(Self { config, z })
    }

    pub fn config(&self) -> &ForecastConfig {
        &self.config
    }

    /// Forecasts every series, backtests the model and ranks recommendations.
    ///
    /// Series with fewer than `min_history_days` daily samples are skipped.
    pub fn run_cycle(
        &self,
        cycle_id: &str,
        now: DateTime<Utc>,
        series: &[UtilizationSeries],
        incidents: &[CapacityIncident],
    ) -> CapacityRecommendationReportSpec {
        let cfg = &self.config;
        let mut forecasts = Vec::new();
        let mut recommendations = Vec::new();
        let mut series_backtests = Vec::new();
        let mut pooled = Score::default();
        let mut history_days = 0u32;
        let mut fitted: Vec<(&UtilizationSeries, DailySeries, ForecastModelKind)> = Vec::new();

        for s in series {
            let daily = DailySeries::from_points(&s.points);
            if daily.len() < cfg.min_history_days as usize {
                warn!(
                    cluster = %s.cluster,
                    dimension = ?s.dimension,
                    samples = daily.len(),
                    "skipping capacity series with insufficient history"
                );
                continue;
            }
            history_days = history_days.max(daily.span_days());

            let linear = self.backtest(&daily, ForecastModelKind::RobustLinear);
            let seasonal = self
                .seasonal_eligible(daily.len())
                .then(|| self.backtest(&daily, ForecastModelKind::SeasonalLinear));
            let model = select_model(&linear, seasonal.as_ref(), cfg.seasonal_min_improvement);
            let chosen = match model {
                ForecastModelKind::SeasonalLinear => seasonal.clone().unwrap_or_default(),
                ForecastModelKind::RobustLinear => linear.clone(),
            };
            debug!(
                cluster = %s.cluster,
                dimension = ?s.dimension,
                ?model,
                linear_mape = ?linear.mape_pct(),
                seasonal_mape = ?seasonal.as_ref().and_then(Score::mape_pct),
                "selected capacity model"
            );
            pooled.merge(&chosen);
            series_backtests.push(SeriesBacktest {
                cluster: s.cluster.clone(),
                dimension: s.dimension,
                object_kind: s.object_kind.clone(),
                selected_model: model,
                linear_mape_pct: linear.mape_pct(),
                seasonal_mape_pct: seasonal.as_ref().and_then(Score::mape_pct),
                interval_coverage: chosen.coverage(),
                samples: chosen.ape_n,
            });

            let end = daily.len();
            let Some(fit) = self.fit_window(&daily, end, model) else {
                continue;
            };
            let last_day = daily.days[end - 1];
            let current_usage = daily.used[end - 1];
            let current_capacity = daily.capacity[end - 1];
            let path = fit.path(last_day, cfg.horizon_days, self.z);
            let limit = current_capacity * cfg.exhaustion_threshold;
            let ttx = time_to_exhaustion(current_usage, limit, &path);
            let at_horizon = *path.last().expect("horizon_days > 0");

            forecasts.push(CapacityForecastSummary {
                cluster: s.cluster.clone(),
                dimension: s.dimension,
                object_kind: s.object_kind.clone(),
                model,
                current_usage,
                current_capacity,
                at_horizon,
                time_to_exhaustion: ttx,
            });

            if let Some(earliest) = ttx.earliest_days {
                let peak_upper = path.iter().map(|p| p.upper).fold(current_usage, f64::max);
                let recommended_capacity =
                    (peak_upper / cfg.target_utilization).max(current_capacity);
                let act_by_day = last_day + i64::from(earliest) - i64::from(cfg.min_lead_time_days);
                let act_by = day_start(act_by_day).max(now);
                recommendations.push(ScalingRecommendation {
                    rank: 0,
                    cluster: s.cluster.clone(),
                    dimension: s.dimension,
                    object_kind: s.object_kind.clone(),
                    priority: self.priority(earliest),
                    current_capacity,
                    recommended_capacity,
                    time_to_exhaustion: ttx,
                    forecast_at_horizon: at_horizon,
                    act_by: act_by.to_rfc3339(),
                    model,
                    rationale: self.rationale(s.dimension, &ttx, model, chosen.mape_pct()),
                });
            }
            fitted.push((s, daily, model));
        }

        rank(&mut recommendations);

        let incident_results: Vec<IncidentBacktest> = incidents
            .iter()
            .map(|incident| self.backtest_incident(incident, &fitted))
            .collect();

        let mape_pct = pooled.mape_pct();
        let backtest = BacktestReport {
            model_version: MODEL_VERSION.to_string(),
            horizon_days: cfg.horizon_days,
            history_days,
            evaluated_forecasts: pooled.ape_n,
            mape_pct,
            interval_coverage: pooled.coverage(),
            mape_target_pct: cfg.mape_target_pct,
            mape_target_met: mape_pct.is_some_and(|m| m < cfg.mape_target_pct),
            min_lead_time_days: cfg.min_lead_time_days,
            lead_time_target_met: incident_results.iter().all(|i| i.met),
            series: series_backtests,
            incidents: incident_results,
        };

        CapacityRecommendationReportSpec {
            cycle_id: cycle_id.to_string(),
            generated_at: now.to_rfc3339(),
            model_version: MODEL_VERSION.to_string(),
            horizon_days: cfg.horizon_days,
            confidence_level: cfg.confidence_level,
            forecasts,
            recommendations,
            backtest,
        }
    }

    fn seasonal_eligible(&self, samples: usize) -> bool {
        // At least three full periods so every slot has repeated observations.
        samples
            >= (self.config.seasonal_period_days as usize * 3)
                .max(self.config.min_history_days as usize)
    }

    /// Fits `kind` on the lookback window ending just before index `end`.
    fn fit_window(&self, daily: &DailySeries, end: usize, kind: ForecastModelKind) -> Option<Fit> {
        let start = end.saturating_sub(self.config.lookback_days as usize);
        let period = self.config.seasonal_period_days as usize;
        Fit::new(
            kind,
            &daily.days[start..end],
            &daily.used[start..end],
            period,
        )
    }

    /// Rolling-origin backtest scored at exactly `horizon_days` ahead.
    fn backtest(&self, daily: &DailySeries, kind: ForecastModelKind) -> Score {
        let cfg = &self.config;
        let horizon = i64::from(cfg.horizon_days);
        let mut score = Score::default();
        let mut end = cfg.min_history_days as usize;
        while end <= daily.len() {
            let origin_day = daily.days[end - 1];
            let target_day = origin_day + horizon;
            if target_day > *daily.days.last().expect("non-empty") {
                break;
            }
            if let (Some(actual), Some(fit)) =
                (daily.used_on(target_day), self.fit_window(daily, end, kind))
            {
                score.add(fit.predict(target_day, self.z), actual);
            }
            end += cfg.backtest_step_days as usize;
        }
        score
    }

    /// Largest lead (up to the horizon) at which the forecast warned that
    /// capacity would be exhausted on or before the incident.
    fn backtest_incident(
        &self,
        incident: &CapacityIncident,
        fitted: &[(&UtilizationSeries, DailySeries, ForecastModelKind)],
    ) -> IncidentBacktest {
        let cfg = &self.config;
        let incident_day = day_index(incident.occurred_at);
        let series = fitted.iter().find(|(s, _, _)| {
            s.cluster == incident.cluster
                && s.dimension == incident.dimension
                && s.object_kind == incident.object_kind
        });

        let lead_time_days = series.and_then(|(_, daily, model)| {
            (1..=cfg.horizon_days).rev().find(|&lead| {
                let origin_day = incident_day - i64::from(lead);
                let end = daily.days.partition_point(|&d| d <= origin_day);
                if end < cfg.min_history_days as usize {
                    return false;
                }
                let Some(fit) = self.fit_window(daily, end, *model) else {
                    return false;
                };
                let last_day = daily.days[end - 1];
                let limit = daily.capacity[end - 1] * cfg.exhaustion_threshold;
                let path = fit.path(last_day, cfg.horizon_days, self.z);
                time_to_exhaustion(daily.used[end - 1], limit, &path)
                    .earliest_days
                    .is_some_and(|d| last_day + i64::from(d) <= incident_day)
            })
        });

        if series.is_none() {
            warn!(
                cluster = %incident.cluster,
                dimension = ?incident.dimension,
                "capacity incident has no matching utilisation series"
            );
        }

        IncidentBacktest {
            cluster: incident.cluster.clone(),
            dimension: incident.dimension,
            object_kind: incident.object_kind.clone(),
            occurred_at: incident.occurred_at.to_rfc3339(),
            lead_time_days,
            met: lead_time_days.is_some_and(|l| l >= cfg.min_lead_time_days),
        }
    }

    fn priority(&self, earliest_days: u32) -> RecommendationPriority {
        let lead = self.config.min_lead_time_days;
        match earliest_days {
            d if d <= lead => RecommendationPriority::Critical,
            d if d <= 30 => RecommendationPriority::High,
            d if d <= 60 => RecommendationPriority::Medium,
            _ => RecommendationPriority::Low,
        }
    }

    fn rationale(
        &self,
        dimension: CapacityDimension,
        ttx: &TimeToExhaustion,
        model: ForecastModelKind,
        mape: Option<f64>,
    ) -> String {
        let fmt_days = |d: Option<u32>| match d {
            Some(d) => d.to_string(),
            None => format!(">{}", self.config.horizon_days),
        };
        let accuracy = mape.map_or_else(
            || "backtest MAPE unavailable".to_string(),
            |m| format!("backtest MAPE {m:.1}%"),
        );
        format!(
            "{dimension:?} usage forecast to reach {:.0}% of capacity in {}-{} days \
             (expected {}) at {:.0}% confidence; {model:?} model, {accuracy}",
            self.config.exhaustion_threshold * 100.0,
            fmt_days(ttx.earliest_days),
            fmt_days(ttx.latest_days),
            fmt_days(ttx.expected_days),
            self.config.confidence_level * 100.0,
        )
    }
}

/// Publishes a cycle's report as a `CapacityRecommendationReport` CR using
/// server-side apply, so re-running a cycle updates the same object.
pub async fn publish_report(
    client: kube::Client,
    namespace: &str,
    name: &str,
    spec: CapacityRecommendationReportSpec,
) -> Result<CapacityRecommendationReport, kube::Error> {
    let api: Api<CapacityRecommendationReport> = Api::namespaced(client, namespace);
    let report = CapacityRecommendationReport::new(name, spec);
    api.patch(
        name,
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(&report),
    )
    .await
}

/// Quarterly cycle identifier, e.g. `2026-Q3`.
pub fn quarter_cycle_id(at: DateTime<Utc>) -> String {
    format!("{}-Q{}", at.year(), at.month0() / 3 + 1)
}

// ---------------------------------------------------------------------------
// Model selection, ranking and exhaustion
// ---------------------------------------------------------------------------

fn select_model(
    linear: &Score,
    seasonal: Option<&Score>,
    min_improvement: f64,
) -> ForecastModelKind {
    match (linear.mape_pct(), seasonal.and_then(Score::mape_pct)) {
        (Some(l), Some(s)) if s < l * (1.0 - min_improvement) => ForecastModelKind::SeasonalLinear,
        _ => ForecastModelKind::RobustLinear,
    }
}

fn rank(recommendations: &mut [ScalingRecommendation]) {
    let key = |r: &ScalingRecommendation| {
        let t = &r.time_to_exhaustion;
        (
            t.earliest_days.unwrap_or(u32::MAX),
            t.expected_days.unwrap_or(u32::MAX),
            t.latest_days.unwrap_or(u32::MAX),
        )
    };
    recommendations.sort_by(|a, b| {
        key(a)
            .cmp(&key(b))
            .then_with(|| a.cluster.cmp(&b.cluster))
            .then_with(|| a.dimension.cmp(&b.dimension))
            .then_with(|| a.object_kind.cmp(&b.object_kind))
    });
    for (i, r) in recommendations.iter_mut().enumerate() {
        r.rank = i as u32 + 1;
    }
}

/// First day (1-based) each band reaches `limit`; `0` if already exhausted.
fn time_to_exhaustion(current: f64, limit: f64, path: &[ForecastInterval]) -> TimeToExhaustion {
    if current >= limit {
        return TimeToExhaustion {
            earliest_days: Some(0),
            expected_days: Some(0),
            latest_days: Some(0),
        };
    }
    let first = |band: fn(&ForecastInterval) -> f64| {
        path.iter()
            .position(|p| band(p) >= limit)
            .map(|i| i as u32 + 1)
    };
    TimeToExhaustion {
        earliest_days: first(|p| p.upper),
        expected_days: first(|p| p.point),
        latest_days: first(|p| p.lower),
    }
}

// ---------------------------------------------------------------------------
// Data preparation
// ---------------------------------------------------------------------------

/// Daily peak usage with the capacity in effect at the day's last sample.
#[derive(Debug, Clone, Default)]
struct DailySeries {
    days: Vec<i64>,
    used: Vec<f64>,
    capacity: Vec<f64>,
}

impl DailySeries {
    fn from_points(points: &[UtilizationPoint]) -> Self {
        let mut buckets: BTreeMap<i64, (f64, DateTime<Utc>, f64)> = BTreeMap::new();
        for p in points {
            if !(p.used.is_finite() && p.capacity.is_finite() && p.used >= 0.0 && p.capacity > 0.0)
            {
                continue;
            }
            buckets
                .entry(day_index(p.timestamp))
                .and_modify(|(used, ts, cap)| {
                    *used = used.max(p.used);
                    if p.timestamp >= *ts {
                        *ts = p.timestamp;
                        *cap = p.capacity;
                    }
                })
                .or_insert((p.used, p.timestamp, p.capacity));
        }
        let mut out = Self::default();
        for (day, (used, _, cap)) in buckets {
            out.days.push(day);
            out.used.push(used);
            out.capacity.push(cap);
        }
        out
    }

    fn len(&self) -> usize {
        self.days.len()
    }

    fn span_days(&self) -> u32 {
        match (self.days.first(), self.days.last()) {
            (Some(f), Some(l)) => (l - f + 1) as u32,
            _ => 0,
        }
    }

    fn used_on(&self, day: i64) -> Option<f64> {
        self.days.binary_search(&day).ok().map(|i| self.used[i])
    }
}

fn day_index(ts: DateTime<Utc>) -> i64 {
    ts.timestamp().div_euclid(SECONDS_PER_DAY)
}

fn day_start(day: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(day * SECONDS_PER_DAY, 0)
        .single()
        .expect("day index within chrono range")
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

/// A fitted trend (+ optional seasonal profile) with a residual scale.
#[derive(Debug, Clone)]
struct Fit {
    slope: f64,
    intercept: f64,
    x0: i64,
    /// Additive seasonal offsets indexed by `day mod period`; empty if linear.
    seasonal: Vec<f64>,
    sigma: f64,
    n: f64,
    x_mean: f64,
    sxx: f64,
}

impl Fit {
    fn new(kind: ForecastModelKind, days: &[i64], values: &[f64], period: usize) -> Option<Self> {
        if days.len() < 2 {
            return None;
        }
        let x0 = days[0];
        let xs: Vec<f64> = days.iter().map(|&d| (d - x0) as f64).collect();

        let (slope, intercept, seasonal) = match kind {
            ForecastModelKind::RobustLinear => {
                let (m, b) = theil_sen(&xs, values);
                (m, b, Vec::new())
            }
            ForecastModelKind::SeasonalLinear => {
                // Trend on raw data, seasonal profile from its residuals,
                // then re-estimate the trend on the deseasonalised series.
                let (m, b) = theil_sen(&xs, values);
                let seasonal = seasonal_profile(days, &xs, values, m, b, period);
                let adjusted: Vec<f64> = days
                    .iter()
                    .zip(values)
                    .map(|(&d, &y)| y - seasonal[slot(d, period)])
                    .collect();
                let (m, b) = theil_sen(&xs, &adjusted);
                (m, b, seasonal)
            }
        };

        let mut fit = Self {
            slope,
            intercept,
            x0,
            seasonal,
            sigma: 0.0,
            n: xs.len() as f64,
            x_mean: xs.iter().sum::<f64>() / xs.len() as f64,
            sxx: 0.0,
        };
        fit.sxx = xs.iter().map(|x| (x - fit.x_mean).powi(2)).sum();
        let residuals: Vec<f64> = days
            .iter()
            .zip(values)
            .map(|(&d, &y)| y - fit.mean(d))
            .collect();
        fit.sigma = MAD_TO_SIGMA * mad(&residuals);
        Some(fit)
    }

    fn mean(&self, day: i64) -> f64 {
        let x = (day - self.x0) as f64;
        let season = if self.seasonal.is_empty() {
            0.0
        } else {
            self.seasonal[slot(day, self.seasonal.len())]
        };
        self.intercept + self.slope * x + season
    }

    /// Point forecast with a prediction interval that widens with leverage.
    fn predict(&self, day: i64, z: f64) -> ForecastInterval {
        let x = (day - self.x0) as f64;
        let leverage = if self.sxx > 0.0 {
            (x - self.x_mean).powi(2) / self.sxx
        } else {
            0.0
        };
        let half = z * self.sigma * (1.0 + 1.0 / self.n + leverage).sqrt();
        let raw = self.mean(day);
        ForecastInterval {
            point: raw.max(0.0),
            lower: (raw - half).max(0.0),
            upper: (raw + half).max(0.0),
        }
    }

    /// Daily forecasts for `last_day + 1 ..= last_day + horizon`.
    fn path(&self, last_day: i64, horizon: u32, z: f64) -> Vec<ForecastInterval> {
        (1..=i64::from(horizon))
            .map(|h| self.predict(last_day + h, z))
            .collect()
    }
}

fn slot(day: i64, period: usize) -> usize {
    day.rem_euclid(period as i64) as usize
}

/// Zero-mean additive profile: median detrended value per slot.
fn seasonal_profile(
    days: &[i64],
    xs: &[f64],
    values: &[f64],
    slope: f64,
    intercept: f64,
    period: usize,
) -> Vec<f64> {
    let mut buckets = vec![Vec::new(); period];
    for ((&d, &x), &y) in days.iter().zip(xs).zip(values) {
        buckets[slot(d, period)].push(y - (intercept + slope * x));
    }
    let mut profile: Vec<f64> = buckets
        .into_iter()
        .map(|b| if b.is_empty() { 0.0 } else { median(b) })
        .collect();
    let mean = profile.iter().sum::<f64>() / period as f64;
    profile.iter_mut().for_each(|v| *v -= mean);
    profile
}

/// Theil–Sen estimator: median pairwise slope, median-residual intercept.
fn theil_sen(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let n = xs.len();
    let mut slopes = Vec::with_capacity(n * n.saturating_sub(1) / 2);
    for i in 0..n {
        for j in i + 1..n {
            let dx = xs[j] - xs[i];
            if dx != 0.0 {
                slopes.push((ys[j] - ys[i]) / dx);
            }
        }
    }
    let slope = if slopes.is_empty() {
        0.0
    } else {
        median(slopes)
    };
    let intercept = median(xs.iter().zip(ys).map(|(x, y)| y - slope * x).collect());
    (slope, intercept)
}

fn median(mut v: Vec<f64>) -> f64 {
    let n = v.len();
    debug_assert!(n > 0);
    let mid = n / 2;
    let (lo, m, _) = v.select_nth_unstable_by(mid, f64::total_cmp);
    let upper = *m;
    if n % 2 == 1 {
        upper
    } else {
        let lower = lo.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (lower + upper) / 2.0
    }
}

fn mad(v: &[f64]) -> f64 {
    let m = median(v.to_vec());
    median(v.iter().map(|x| (x - m).abs()).collect())
}

/// Standard normal quantile for `p` in (0.5, 1) (Abramowitz & Stegun
/// 26.2.23, absolute error < 4.5e-4).
fn normal_quantile(p: f64) -> f64 {
    let t = (-2.0 * (1.0 - p).ln()).sqrt();
    t - (2.515517 + 0.802853 * t + 0.010328 * t * t)
        / (1.0 + 1.432788 * t + 0.189269 * t * t + 0.001308 * t * t * t)
}

// ---------------------------------------------------------------------------
// Backtest scoring
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Score {
    ape_sum: f64,
    ape_n: u32,
    covered: u32,
    scored: u32,
}

impl Score {
    fn add(&mut self, forecast: ForecastInterval, actual: f64) {
        self.scored += 1;
        if (forecast.lower..=forecast.upper).contains(&actual) {
            self.covered += 1;
        }
        // APE is undefined at zero usage; those points still count for coverage.
        if actual > f64::EPSILON {
            self.ape_sum += (forecast.point - actual).abs() / actual;
            self.ape_n += 1;
        }
    }

    fn merge(&mut self, other: &Score) {
        self.ape_sum += other.ape_sum;
        self.ape_n += other.ape_n;
        self.covered += other.covered;
        self.scored += other.scored;
    }

    fn mape_pct(&self) -> Option<f64> {
        (self.ape_n > 0).then(|| 100.0 * self.ape_sum / f64::from(self.ape_n))
    }

    fn coverage(&self) -> Option<f64> {
        (self.scored > 0).then(|| f64::from(self.covered) / f64::from(self.scored))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// Deterministic noise in [-1, 1].
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        }
    }

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 9, 1, 0, 0, 0).unwrap()
    }

    fn series(
        cluster: &str,
        dimension: CapacityDimension,
        days: i64,
        mut f: impl FnMut(i64) -> (f64, f64),
    ) -> UtilizationSeries {
        let points = (0..days)
            .flat_map(|d| {
                let (used, capacity) = f(d);
                // Several samples per day; the daily peak is `used`.
                [0.6, 0.9, 1.0].map(|k| UtilizationPoint {
                    timestamp: start() + Duration::days(d) + Duration::hours((k * 20.0) as i64),
                    used: used * k,
                    capacity,
                })
            })
            .collect();
        UtilizationSeries {
            cluster: cluster.into(),
            dimension,
            object_kind: None,
            points,
        }
    }

    fn engine() -> CapacityForecastEngine {
        CapacityForecastEngine::new(ForecastConfig::default()).unwrap()
    }

    fn now(days: i64) -> DateTime<Utc> {
        start() + Duration::days(days)
    }

    #[test]
    fn theil_sen_ignores_outliers() {
        let xs: Vec<f64> = (0..50).map(f64::from).collect();
        let mut ys: Vec<f64> = xs.iter().map(|x| 2.0 * x + 5.0).collect();
        ys[10] = 1_000.0;
        ys[30] = -500.0;
        ys[45] = 10_000.0;
        let (m, b) = theil_sen(&xs, &ys);
        assert!((m - 2.0).abs() < 1e-9, "slope {m}");
        assert!((b - 5.0).abs() < 1e-9, "intercept {b}");
    }

    #[test]
    fn normal_quantile_matches_known_values() {
        assert!((normal_quantile(0.95) - 1.6449).abs() < 1e-3);
        assert!((normal_quantile(0.975) - 1.9600).abs() < 1e-3);
    }

    #[test]
    fn config_validation_rejects_bad_values() {
        let bad = [
            ForecastConfig {
                horizon_days: 0,
                ..Default::default()
            },
            ForecastConfig {
                confidence_level: 1.0,
                ..Default::default()
            },
            ForecastConfig {
                lookback_days: 10,
                ..Default::default()
            },
            ForecastConfig {
                target_utilization: 0.0,
                ..Default::default()
            },
            ForecastConfig {
                exhaustion_threshold: 1.5,
                ..Default::default()
            },
        ];
        for cfg in bad {
            assert!(matches!(
                CapacityForecastEngine::new(cfg),
                Err(ForecastError::InvalidConfig(_))
            ));
        }
    }

    #[test]
    fn noisy_linear_growth_meets_mape_target_with_linear_model() {
        let mut rng = Lcg(7);
        let s = series("c1", CapacityDimension::Cpu, 365, |d| {
            (100.0 + 0.5 * d as f64 + 4.0 * rng.next(), 1_000.0)
        });
        let report = engine().run_cycle("2026-Q3", now(365), &[s], &[]);
        let bt = &report.backtest;
        assert!(bt.mape_target_met, "mape {:?}", bt.mape_pct);
        assert!(bt.evaluated_forecasts > 20);
        assert_eq!(bt.series[0].selected_model, ForecastModelKind::RobustLinear);
        assert!(bt.interval_coverage.unwrap() > 0.7);
        assert_eq!(bt.model_version, MODEL_VERSION);
    }

    #[test]
    fn strong_weekly_pattern_escalates_to_seasonal_model() {
        let mut rng = Lcg(11);
        let s = series("c1", CapacityDimension::Memory, 365, |d| {
            let weekly = if d.rem_euclid(7) < 5 { 60.0 } else { -60.0 };
            (300.0 + 0.4 * d as f64 + weekly + 3.0 * rng.next(), 2_000.0)
        });
        let report = engine().run_cycle("2026-Q3", now(365), &[s], &[]);
        let sb = &report.backtest.series[0];
        assert_eq!(sb.selected_model, ForecastModelKind::SeasonalLinear);
        assert!(sb.seasonal_mape_pct.unwrap() < sb.linear_mape_pct.unwrap());
        assert!(report.backtest.mape_target_met);
    }

    #[test]
    fn each_dimension_is_forecast_separately_with_intervals() {
        let dims = [
            CapacityDimension::Cpu,
            CapacityDimension::Memory,
            CapacityDimension::Storage,
            CapacityDimension::ObjectCount,
        ];
        let input: Vec<_> = dims
            .iter()
            .enumerate()
            .map(|(i, &dim)| {
                let mut rng = Lcg(i as u64 + 1);
                let rate = (i + 1) as f64;
                series("c1", dim, 200, move |d| {
                    (50.0 + rate * d as f64 + rng.next(), 1e6)
                })
            })
            .collect();
        let report = engine().run_cycle("2026-Q3", now(200), &input, &[]);
        assert_eq!(report.forecasts.len(), 4);
        for (f, dim) in report.forecasts.iter().zip(dims) {
            assert_eq!(f.dimension, dim);
            let ci = f.at_horizon;
            assert!(ci.lower <= ci.point && ci.point <= ci.upper && ci.upper > ci.lower);
        }
        // Different growth rates give different forecasts.
        assert!(report.forecasts[3].at_horizon.point > report.forecasts[0].at_horizon.point);
    }

    #[test]
    fn recommendations_are_ranked_by_time_to_exhaustion() {
        let mk = |name: &str, base: f64, rate: f64, seed: u64| {
            let mut rng = Lcg(seed);
            series(name, CapacityDimension::Storage, 180, move |d| {
                (base + rate * d as f64 + rng.next(), 1_000.0)
            })
        };
        // Capacity 1000: fast exhausts in ~20 days, mid in ~70, slow and flat
        // not within the 90-day horizon.
        let input = [
            mk("slow", 400.0, 1.0, 1),
            mk("mid", 750.0, 1.0, 2),
            mk("flat", 500.0, 0.0, 3),
            mk("fast", 800.0, 1.0, 4),
        ];
        let report = engine().run_cycle("2026-Q3", now(180), &input, &[]);
        let order: Vec<_> = report
            .recommendations
            .iter()
            .map(|r| r.cluster.as_str())
            .collect();
        assert_eq!(
            order,
            ["fast", "mid"],
            "flat/slow must not exhaust within 90 days"
        );
        assert_eq!(report.recommendations[0].rank, 1);
        assert_eq!(report.recommendations[1].rank, 2);

        let top = &report.recommendations[0];
        let t = top.time_to_exhaustion;
        assert!(t.earliest_days.unwrap() <= t.expected_days.unwrap());
        assert!(top.recommended_capacity * 0.8 >= top.forecast_at_horizon.upper - 1e-6);
        assert!(top.recommended_capacity > top.current_capacity);
    }

    #[test]
    fn already_exhausted_series_is_critical_now() {
        let s = series("c1", CapacityDimension::Cpu, 60, |d| {
            (90.0 + d as f64, 100.0)
        });
        let report = engine().run_cycle("2026-Q3", now(60), &[s], &[]);
        let r = &report.recommendations[0];
        assert_eq!(r.time_to_exhaustion.earliest_days, Some(0));
        assert_eq!(r.priority, RecommendationPriority::Critical);
        assert_eq!(r.act_by, now(60).to_rfc3339());
    }

    #[test]
    fn historical_incident_is_predicted_with_required_lead_time() {
        // Usage crosses capacity (500) on day 300; the autoscaler then doubles it.
        let mut rng = Lcg(3);
        let s = series("c1", CapacityDimension::Memory, 365, |d| {
            let cap = if d < 300 { 500.0 } else { 1_000.0 };
            (200.0 + 1.0 * d as f64 + 3.0 * rng.next(), cap)
        });
        let incident = CapacityIncident {
            cluster: "c1".into(),
            dimension: CapacityDimension::Memory,
            object_kind: None,
            occurred_at: now(300),
        };
        let missing = CapacityIncident {
            cluster: "unknown".into(),
            ..incident.clone()
        };
        let report = engine().run_cycle("2026-Q3", now(365), &[s], std::slice::from_ref(&incident));
        let ib = &report.backtest.incidents[0];
        assert!(ib.met && ib.lead_time_days.unwrap() >= 14, "{ib:?}");
        assert!(report.backtest.lead_time_target_met);

        let s = series("c1", CapacityDimension::Memory, 365, |d| {
            (200.0 + d as f64, 1_000.0)
        });
        let report = engine().run_cycle("2026-Q3", now(365), &[s], &[missing]);
        assert!(!report.backtest.incidents[0].met);
        assert!(!report.backtest.lead_time_target_met);
    }

    #[test]
    fn short_series_are_skipped() {
        let s = series("c1", CapacityDimension::Cpu, 10, |d| (d as f64, 100.0));
        let report = engine().run_cycle("2026-Q3", now(10), &[s], &[]);
        assert!(report.forecasts.is_empty());
        assert!(report.backtest.mape_pct.is_none());
        assert!(!report.backtest.mape_target_met);
    }

    #[test]
    fn quarter_cycle_ids() {
        assert_eq!(
            quarter_cycle_id(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            "2026-Q1"
        );
        assert_eq!(
            quarter_cycle_id(Utc.with_ymd_and_hms(2026, 9, 26, 0, 0, 0).unwrap()),
            "2026-Q3"
        );
        assert_eq!(
            quarter_cycle_id(Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap()),
            "2026-Q4"
        );
    }

    #[test]
    fn report_exports_as_custom_resource() {
        use kube::{CustomResourceExt, Resource};
        let mut rng = Lcg(5);
        let s = series("c1", CapacityDimension::Cpu, 200, |d| {
            (100.0 + d as f64 + rng.next(), 300.0)
        });
        let spec = engine().run_cycle("2026-Q3", now(200), &[s], &[]);
        let cr = CapacityRecommendationReport::new("capacity-2026-q3", spec);
        assert_eq!(
            CapacityRecommendationReport::kind(&()),
            "CapacityRecommendationReport"
        );
        let json = serde_json::to_value(&cr).unwrap();
        assert_eq!(json["apiVersion"], "stellar.org/v1alpha1");
        assert_eq!(json["spec"]["recommendations"][0]["rank"], 1);
        assert!(json["spec"]["backtest"]["mapePct"].is_number());
        let back: CapacityRecommendationReport = serde_json::from_value(json).unwrap();
        assert_eq!(back.spec, cr.spec);
        assert_eq!(
            CapacityRecommendationReport::crd().metadata.name.as_deref(),
            Some("capacityrecommendationreports.stellar.org")
        );
    }
}
