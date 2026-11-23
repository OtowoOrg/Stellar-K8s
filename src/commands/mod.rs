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
//! Subcommand implementations for the Stellar-K8s operator CLI.
//!
//! Each module corresponds to a major functional area of the operator's
//! command-line interface, such as running the operator, the simulator,
//! or generating runbooks.

pub mod backup;
pub mod benchmark;
pub mod check_crd;
pub mod doctor;
pub mod export_compliance;
pub mod health_check;
pub mod info;
pub mod operator;
pub mod runbook;
pub mod schema_compat;
pub mod simulator;
pub mod webhook;

use clap::Subcommand;

/// Top-level CLI subcommands supported by the operator binary.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run the Stellar-K8s operator.
    Operator(operator::OperatorArgs),
    /// Run the cluster simulator.
    Simulator(simulator::SimulatorArgs),
    /// Generate operational runbooks.
    Runbook(runbook::RunbookArgs),
    /// Check CRD definitions for correctness.
    CheckCrd(check_crd::CheckCrdArgs),
    /// Run diagnostics against a cluster.
    Doctor(doctor::DoctorArgs),
    /// Export compliance reports.
    ExportCompliance(export_compliance::ExportComplianceArgs),
    /// Perform a health check.
    HealthCheck(health_check::HealthCheckArgs),
    /// Print build and version information.
    Info(info::InfoArgs),
    /// Create or restore backups.
    Backup(backup::BackupArgs),
    /// Run performance benchmarks.
    Benchmark(benchmark::BenchmarkArgs),
    /// Validate admission webhooks.
    Webhook(webhook::WebhookArgs),
    /// Check schema compatibility against registered consumers.
    SchemaCompat(schema_compat::SchemaCompatArgs),
}

impl Commands {
    /// Dispatch the selected subcommand to its implementation.
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            Commands::Operator(args) => operator::run(args).await,
            Commands::Simulator(args) => simulator::run(args).await,
            Commands::Runbook(args) => runbook::run(args).await,
            Commands::CheckCrd(args) => check_crd::run(args).await,
            Commands::Doctor(args) => doctor::run(args).await,
            Commands::ExportCompliance(args) => export_compliance::run(args).await,
            Commands::HealthCheck(args) => health_check::run(args).await,
            Commands::Info(args) => info::run(args).await,
            Commands::Backup(args) => backup::run(args).await,
            Commands::Benchmark(args) => benchmark::run(args).await,
            Commands::Webhook(args) => webhook::run(args).await,
            Commands::SchemaCompat(args) => schema_compat::run(args).await,
        }
    }
}