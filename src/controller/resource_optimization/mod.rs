//! Dynamic resource optimization with ML-based workload prediction.
//!
//! Provides time-series forecasting, SLA-aware predictive autoscaling,
//! intelligent vertical pod autoscaling recommendations, and what-if simulation.

pub mod controller;
pub mod forecasting;
pub mod metrics;
pub mod simulation;
pub mod sla;
pub mod vpa_optimizer;

pub use controller::{
    OptimizationController, OptimizationRecommendation, ResourceOptimizationConfig,
};
pub use forecasting::{
    ForecastEngine, ForecastModel, ForecastPoint, ForecastResult, TimeSeriesPoint,
};
pub use simulation::{CapacitySimulator, SimulationResult, SimulationScenario};
pub use sla::{SlaConstraint, SlaEvaluator, SlaMetrics, SlaViolation};
pub use vpa_optimizer::{VpaOptimization, VpaRecommendation};
