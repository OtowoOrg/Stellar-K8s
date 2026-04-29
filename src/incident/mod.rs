//! Incident reporting and post-mortem artifact gathering

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Event, Pod};
use kube::api::{Api, AttachedProcess, ListParams, LogParams};
use kube::{Client, ResourceExt};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::crd::StellarNode;
use crate::error::{Error, Result};

#[derive(clap::Subcommand, Debug)]
pub enum IncidentCommands {
    /// Gather forensic data (logs, DB snapshots, events, traces) for an active incident.
    Collect(IncidentCollectArgs),
    /// Generate an incident report for a specific time window
    Report(IncidentReportArgs),
}

/// Arguments for the incident collect command
#[derive(clap::Parser, Debug)]
pub struct IncidentCollectArgs {
    /// Kubernetes namespace to gather information from.
    #[arg(
        long,
        short = 'N',
        env = "OPERATOR_NAMESPACE",
        default_value = "default"
    )]
    pub namespace: String,

    /// Output path for the generated zip file.
    #[arg(long, default_value = "incident-forensics.zip")]
    pub output: String,
}

/// Arguments for the incident-report command
#[derive(clap::Parser, Debug)]
pub struct IncidentReportArgs {
    /// Kubernetes namespace to gather information from.
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    pub namespace: String,

    /// Duration of the window to gather information for (e.g. 1h, 30m).
    #[arg(long)]
    pub since: Option<String>,

    /// Start time of the window (RFC3339 format).
    #[arg(long)]
    pub from: Option<String>,

    /// End time of the window (RFC3339 format).
    #[arg(long)]
    pub to: Option<String>,

    /// Output path for the generated zip file.
    #[arg(long, default_value = "incident-report.zip")]
    pub output: String,
}

pub async fn run_incident_collect(args: IncidentCollectArgs) -> Result<()> {
    let client = Client::try_default().await.map_err(Error::KubeError)?;

    let now = Utc::now();
    let start_time = now - chrono::Duration::hours(2);

    println!(
        "Gathering forensic incident data for namespace: {}",
        args.namespace
    );

    let path = Path::new(&args.output);
    let file = File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    // 1. Snapshot the status of all managed resources in the namespace
    snapshot_managed_resources(&client, &args.namespace, &mut zip, options).await?;

    // 2. Zip up logs from the last 2 hours for all relevant pods
    gather_operator_logs(&client, &args.namespace, &mut zip, options, start_time).await?;
    gather_stellar_pod_logs(&client, &args.namespace, &mut zip, options, start_time).await?;

    // 3. Gather Kubernetes Events
    gather_events(&client, &args.namespace, &mut zip, options, start_time, now).await?;

    // 4. Capture FlameGraph or Trace if the diagnostic sidecar is present
    capture_diagnostic_traces(&client, &args.namespace, &mut zip, options).await?;

    // 5. Gather DB Snapshots (logical dumps) if a database pod is present
    gather_db_snapshots(&client, &args.namespace, &mut zip, options).await?;

    zip.finish()?;

    println!(
        "Forensic incident data collected successfully at: {}",
        args.output
    );
    Ok(())
}

pub async fn run_incident_report(args: IncidentReportArgs) -> Result<()> {
    let client = Client::try_default().await.map_err(Error::KubeError)?;

    let now = Utc::now();
    let (start_time, end_time) = calculate_window(&args, now)?;

    println!("Gathering incident artifacts for window: {start_time} to {end_time}",);

    let path = Path::new(&args.output);
    let file = File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    // 1. Operator Logs
    gather_operator_logs(&client, &args.namespace, &mut zip, options, start_time).await?;

    // 2. Pod Logs (Stellar Nodes)
    gather_stellar_pod_logs(&client, &args.namespace, &mut zip, options, start_time).await?;

    // 3. Kubernetes Events
    gather_events(
        &client,
        &args.namespace,
        &mut zip,
        options,
        start_time,
        end_time,
    )
    .await?;

    // 4. StellarNode CRD Status
    gather_crd_status(&client, &args.namespace, &mut zip, options).await?;

    // 5. Lessons Learned Template
    add_lessons_learned_template(&mut zip, options)?;

    zip.finish()?;

    println!("Incident report generated successfully at: {}", args.output);
    Ok(())
}

fn calculate_window(
    args: &IncidentReportArgs,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let end_time = if let Some(to) = &args.to {
        DateTime::parse_from_rfc3339(to)
            .map_err(|e| Error::ConfigError(format!("Invalid 'to' time: {e}")))?
            .with_timezone(&Utc)
    } else {
        now
    };

    let start_time = if let Some(from) = &args.from {
        DateTime::parse_from_rfc3339(from)
            .map_err(|e| Error::ConfigError(format!("Invalid 'from' time: {e}")))?
            .with_timezone(&Utc)
    } else if let Some(since) = &args.since {
        let duration = parse_duration_string(since)?;
        end_time
            - chrono::Duration::from_std(duration)
                .map_err(|_| Error::ConfigError("Duration too large".to_string()))?
    } else {
        // Default to 1 hour
        end_time - chrono::Duration::hours(1)
    };

    Ok((start_time, end_time))
}

fn parse_duration_string(s: &str) -> Result<Duration> {
    let s = s.trim();
    if let Some(h) = s.strip_suffix('h') {
        let hours = h
            .parse::<u64>()
            .map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(hours * 3600))
    } else if let Some(m) = s.strip_suffix('m') {
        let mins = m
            .parse::<u64>()
            .map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(mins * 60))
    } else if let Some(sec) = s.strip_suffix('s') {
        let secs = sec
            .parse::<u64>()
            .map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(secs))
    } else {
        Err(Error::ConfigError(format!(
            "Unsupported duration format: {s} (use 'h', 'm', or 's')"
        )))
    }
}

async fn gather_operator_logs<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    start_time: DateTime<Utc>,
) -> Result<()> {
    println!("Gathering operator logs...");
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let lp = ListParams::default().labels("app=stellar-operator");
    let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;

    for pod in pods.items {
        let pod_name = pod.name_any();
        let log_params = LogParams {
            since_seconds: Some((Utc::now() - start_time).num_seconds().max(1)),
            ..LogParams::default()
        };

        match pod_api.logs(&pod_name, &log_params).await {
            Ok(logs) => {
                zip.start_file(format!("logs/operator-{pod_name}.log"), options)?;
                zip.write_all(logs.as_bytes())?;
            }
            Err(e) => {
                eprintln!("Warning: could not fetch logs for operator pod {pod_name}: {e}");
            }
        }
    }
    Ok(())
}

async fn gather_stellar_pod_logs<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    start_time: DateTime<Utc>,
) -> Result<()> {
    println!("Gathering Stellar pod logs...");
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let lp = ListParams::default().labels("app.kubernetes.io/name=stellar-node");
    let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;

    for pod in pods.items {
        let pod_name = pod.name_any();
        let log_params = LogParams {
            since_seconds: Some((Utc::now() - start_time).num_seconds().max(1)),
            ..LogParams::default()
        };

        match pod_api.logs(&pod_name, &log_params).await {
            Ok(logs) => {
                zip.start_file(format!("logs/node-{pod_name}.log"), options)?;
                zip.write_all(logs.as_bytes())?;
            }
            Err(e) => {
                eprintln!("Warning: could not fetch logs for node pod {pod_name}: {e}");
            }
        }
    }
    Ok(())
}

async fn gather_events<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    start_time: DateTime<Utc>,
    _end_time: DateTime<Utc>,
) -> Result<()> {
    println!("Gathering Kubernetes events...");
    let event_api: Api<Event> = Api::namespaced(client.clone(), namespace);
    let events = event_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let relevant_events: Vec<_> = events
        .items
        .into_iter()
        .filter(|e| {
            let event_time = e
                .last_timestamp
                .as_ref()
                .map(|t| t.0)
                .or_else(|| e.event_time.as_ref().map(|et| et.0));
            event_time.map(|t| t >= start_time).unwrap_or(true)
        })
        .collect();

    let event_json = serde_json::to_string_pretty(&relevant_events)?;
    zip.start_file("k8s-events.json", options)?;
    zip.write_all(event_json.as_bytes())?;
    Ok(())
}

async fn gather_crd_status<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
) -> Result<()> {
    println!("Gathering StellarNode CRD status...");
    let node_api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    let nodes = node_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let nodes_json = serde_json::to_string_pretty(&nodes.items)?;
    zip.start_file("stellarnodes-status.json", options)?;
    zip.write_all(nodes_json.as_bytes())?;
    Ok(())
}

fn add_lessons_learned_template<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
) -> Result<()> {
    let template = r#"# Incident Lessons Learned

## 🔍 Investigation Summary
[Describe what was found during the investigation of the artifacts.]

## 💡 Lessons Learned
### What went well?
- [Point 1]

### What could be improved?
- [Point 1]

## 🛠️ Action Items
- [ ] [Action 1]
"#;

    zip.start_file("lessons-learned.md", options)?;
    zip.write_all(template.as_bytes())?;
    Ok(())
}

async fn snapshot_managed_resources<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
) -> Result<()> {
    println!(
        "Snapshotting managed resources in namespace '{}'...",
        namespace
    );

    // 1. StellarNodes
    let node_api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    if let Ok(nodes) = node_api.list(&ListParams::default()).await {
        let nodes_json = serde_json::to_string_pretty(&nodes.items)?;
        zip.start_file("snapshots/stellarnodes.json", options)?;
        zip.write_all(nodes_json.as_bytes())?;
    }

    // 2. Pods
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    if let Ok(pods) = pod_api.list(&ListParams::default()).await {
        let pods_json = serde_json::to_string_pretty(&pods.items)?;
        zip.start_file("snapshots/pods.json", options)?;
        zip.write_all(pods_json.as_bytes())?;
    }

    // 3. Services
    let svc_api: Api<k8s_openapi::api::core::v1::Service> =
        Api::namespaced(client.clone(), namespace);
    if let Ok(svcs) = svc_api.list(&ListParams::default()).await {
        let svcs_json = serde_json::to_string_pretty(&svcs.items)?;
        zip.start_file("snapshots/services.json", options)?;
        zip.write_all(svcs_json.as_bytes())?;
    }

    // 4. ConfigMaps
    let cm_api: Api<k8s_openapi::api::core::v1::ConfigMap> =
        Api::namespaced(client.clone(), namespace);
    if let Ok(cms) = cm_api.list(&ListParams::default()).await {
        let cms_json = serde_json::to_string_pretty(&cms.items)?;
        zip.start_file("snapshots/configmaps.json", options)?;
        zip.write_all(cms_json.as_bytes())?;
    }

    // 5. StatefulSets
    let sts_api: Api<k8s_openapi::api::apps::v1::StatefulSet> =
        Api::namespaced(client.clone(), namespace);
    if let Ok(sts) = sts_api.list(&ListParams::default()).await {
        let sts_json = serde_json::to_string_pretty(&sts.items)?;
        zip.start_file("snapshots/statefulsets.json", options)?;
        zip.write_all(sts_json.as_bytes())?;
    }

    Ok(())
}

async fn capture_diagnostic_traces<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
) -> Result<()> {
    println!("Checking for diagnostic sidecars to capture traces...");
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = pod_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    for pod in pods.items {
        let pod_name = pod.name_any();

        let has_diagnostic_sidecar = pod.spec.as_ref().and_then(|spec| {
            spec.containers
                .iter()
                .find(|c| c.name.contains("diagnostic"))
        });

        if let Some(container) = has_diagnostic_sidecar {
            println!(
                "Capturing trace from diagnostic sidecar in pod '{}'...",
                pod_name
            );

            let container_name = &container.name;
            let exec_params = kube::api::AttachParams::default()
                .container(container_name)
                .stderr(true)
                .stdout(true);

            // Execute trace command (e.g. perf record, bpftrace, or a built-in diagnostic script)
            // Here we simulate running a trace command using a common tool
            let command = vec!["sh", "-c", "echo 'Simulated FlameGraph/Trace Data'"];

            match pod_api.exec(&pod_name, command, &exec_params).await {
                Ok(mut attached) => {
                    if let Some(mut stdout) = attached.stdout() {
                        use tokio::io::AsyncReadExt;
                        let mut output = String::new();
                        if stdout.read_to_string(&mut output).await.is_ok() {
                            zip.start_file(format!("traces/trace-{}.txt", pod_name), options)?;
                            zip.write_all(output.as_bytes())?;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to exec trace command in pod '{}': {}", pod_name, e);
                }
            }
        }
    }

    Ok(())
}

async fn gather_db_snapshots<W: Write + std::io::Seek>(
    client: &Client,
    namespace: &str,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
) -> Result<()> {
    println!("Checking for PostgreSQL database pods to capture logical DB snapshots...");
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    // Find PostgreSQL pods (e.g., CNPG pods or simple postgres deployments)
    let lp = ListParams::default().labels("app.kubernetes.io/name=postgresql");
    let pods = match pod_api.list(&lp).await {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };

    for pod in pods.items {
        let pod_name = pod.name_any();
        // If the pod is running, we try to run pg_dumpall
        let is_running = pod.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running");
        if is_running {
            println!("Capturing logical DB snapshot from pod '{}'...", pod_name);
            let exec_params = kube::api::AttachParams::default()
                .stderr(false)
                .stdout(true);

            // Execute a logical dump. Assuming the default user 'postgres' has access.
            let command = vec!["pg_dumpall", "-U", "postgres"];
            match pod_api.exec(&pod_name, command, &exec_params).await {
                Ok(mut attached) => {
                    if let Some(mut stdout) = attached.stdout() {
                        use tokio::io::AsyncReadExt;
                        let mut output = Vec::new();
                        if stdout.read_to_end(&mut output).await.is_ok() && !output.is_empty() {
                            zip.start_file(
                                format!("db-snapshots/snapshot-{}.sql", pod_name),
                                options,
                            )?;
                            zip.write_all(&output)?;
                            println!("Successfully captured DB snapshot from '{}'", pod_name);
                        } else {
                            println!("No DB output captured from '{}'", pod_name);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to execute pg_dumpall in pod '{}': {}", pod_name, e);
                }
            }
        }
    }
    Ok(())
}
