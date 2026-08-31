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
use crate::cli::BenchmarkArgs;
use crate::logging::{init_subscriber, LogOutputFormat, SubscriberConfig};
use crate::Error;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing::info;

pub async fn run_benchmark_controller_cmd(args: BenchmarkArgs) -> Result<(), Error> {
    use crate::controller::run_benchmark_controller;

    init_subscriber(SubscriberConfig::from_level_str(
        &args.log_level,
        LogOutputFormat::Json,
    ));
    // Minimal tracing setup for the benchmark controller.
    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(
            args.log_level
                .parse()
                .unwrap_or(tracing::Level::INFO.into()),
        )
        .from_env_lossy();

    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_target(true)
        .with_env_filter(env_filter)
        .init();

    info!(
        "Starting StellarBenchmark controller v{}",
        env!("CARGO_PKG_VERSION")
    );

    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    // The benchmark controller always acts as leader (it is stateless and
    // idempotent, so multiple replicas are safe).
    let is_leader = Arc::new(AtomicBool::new(true));

    run_benchmark_controller(client, is_leader)
        .await
        .map_err(|e| Error::ConfigError(format!("Benchmark controller error: {e}")))?;

    Ok(())
}
