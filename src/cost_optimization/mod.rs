//! Advanced Cost Optimization with Multi-Cloud Pricing Analysis
//!
//! Real-time cost tracking, anomaly detection, optimization recommendations,
//! reserved/spot instance analysis, cost allocation, and forecasting.

pub mod allocation;
pub mod anomaly;
pub mod calculator;
pub mod dashboard;
pub mod forecast;
pub mod model;
pub mod recommender;

pub use allocation::{CostAllocation, NamespaceCost};
pub use anomaly::{AnomalyDetector, CostAnomaly};
pub use calculator::{CloudCostCalculator, ResourceCost};
pub use dashboard::CostDashboard;
pub use forecast::{CostForecast, CostForecaster};
pub use model::{CloudProvider, CostRecord, ResourceType};
pub use recommender::{OptimizationRecommendation, RecommendationEngine};
