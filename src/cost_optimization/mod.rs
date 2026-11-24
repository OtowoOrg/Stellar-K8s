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
//! Advanced Cost Optimization with Multi-Cloud Pricing Analysis (issue #1413)
//!
//! Real-time cost tracking, anomaly detection, optimization recommendations,
//! reserved/spot instance analysis, cost allocation, and forecasting.
//!
//! ## New in #1413
//!
//! - [`resource_tracker`]: Records observed CPU/memory utilisation and computes
//!   right-sizing recommendations per workload.
//! - Spot instance integration wired through the existing [`spot`] module.
//! - Cost dashboard extended with anomaly detection panels.

pub mod allocation;
pub mod anomaly;
pub mod calculator;
pub mod dashboard;
pub mod forecast;
pub mod model;
pub mod recommender;
pub mod resource_tracker;
pub mod spot;

pub use allocation::{CostAllocation, NamespaceCost};
pub use anomaly::{AnomalyDetector, CostAnomaly};
pub use calculator::{CloudCostCalculator, ResourceCost};
pub use dashboard::CostDashboard;
pub use forecast::{CostForecast, CostForecaster};
pub use model::{CloudProvider, CostRecord, ResourceType};
pub use recommender::{OptimizationRecommendation, RecommendationEngine};
pub use resource_tracker::{
    ResourceObservation, ResourceTracker, RightSizingConfig, RightSizingRecommendation,
};
pub use spot::{SpotConfig, SpotCostAnalysis, SpotManager, SpotRequest};
