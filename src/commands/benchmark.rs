use crate::cli::BenchmarkArgs;
use crate::Error;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use stellar_k8s::logging::{init_subscriber, LogOutputFormat, SubscriberConfig};
use tracing::info;

pub async fn run_benchmark_controller_cmd(args: BenchmarkArgs) -> Result<(), Error> {
    use stellar_k8s::controller::run_benchmark_controller;

    init_subscriber(SubscriberConfig::from_level_str(
        &args.log_level,
        LogOutputFormat::Json,
    ));

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
