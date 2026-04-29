use crate::cli::BenchmarkArgs;
use crate::Error;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing::info;

pub async fn run_benchmark_controller_cmd(args: BenchmarkArgs) -> Result<(), Error> {
    use stellar_k8s::controller::run_benchmark_controller;

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
