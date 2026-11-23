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


pub mod bloat;
pub mod compactor;
pub mod controller;
pub mod coordinator;
pub mod db;
pub mod node_drain;
pub mod pruner;
pub mod plan_controller;
pub mod plan_engine;
pub mod query_profiler;
pub mod readiness;
pub mod vacuum;

pub use bloat::BloatDetector;
pub use compactor::{
    run_compaction_cycle, CompactionCoordinator, CompactionDaemon, CompactionGuard,
    CompactionReport, COMPACTION_MARKER_ANNOTATION,
};
pub use controller::MaintenanceController;
pub use coordinator::MaintenanceCoordinator;
pub use db::{
    evaluate_fragmentation, total_relation_size, verify_integrity, DatabaseIntegrityVerifier,
    FragmentationMetrics, IntegrityReport, LedgerPruner, PruningReport,
};
pub use node_drain::NodeDrainOrchestrator;
pub use pruner::{Pruner, PrunerConfig, PruningResult, run_pruner_controller};
pub use plan_controller::run_maintenance_plan_controller;
pub use plan_engine::{reconcile_plan, SimulatedCluster};
pub use query_profiler::{IndexSuggestion, QueryProfiler, SlowQuery};
pub use vacuum::{run_vacuum_controller, DefragResult, VacuumConfig, VacuumDefrag};
