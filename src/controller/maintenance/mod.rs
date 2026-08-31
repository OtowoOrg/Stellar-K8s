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
//! Maintenance Window controller for Horizon DB maintenance tasks.
//!
//! Handles scheduling and coordination of VACUUM FULL and REINDEX operations.

pub mod bloat;
pub mod controller;
pub mod coordinator;
pub mod node_drain;
pub mod query_profiler;

pub use bloat::BloatDetector;
pub use controller::MaintenanceController;
pub use coordinator::MaintenanceCoordinator;
pub use node_drain::NodeDrainOrchestrator;
pub use query_profiler::{IndexSuggestion, QueryProfiler, SlowQuery};
