//! Incident reporting and post-mortem artifact gathering

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Event, Pod};
use kube::api::{Api, ListParams, LogParams};
use kube::{Client, ResourceExt};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::crd::StellarNode;
use crate::error::{Error, Result};

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
