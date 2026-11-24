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
