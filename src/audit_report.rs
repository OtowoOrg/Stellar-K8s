use aws_sdk_s3::Client as S3Client;
use comfy_table::Table;
use serde::{Deserialize, Serialize};
use tracing::error;

use stellar_k8s::controller::audit_log::AuditEntry;
use stellar_k8s::error::{Error, Result};

/// JSON-serializable report wrapping a list of audit entries.
///
/// Emitted when `--json` is passed to `kubectl stellar audit list`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditReport {
    pub timestamp: String,
    pub cluster_name: String,
    pub results: Vec<AuditResult>,
}

/// A single audit result item inside [`AuditReport`].
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditResult {
    pub check_name: String,
    pub status: AuditStatus,
    pub details: Option<String>,
    pub remediation: Option<String>,
}

/// Pass/Fail status for an audit result.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AuditStatus {
    Pass,
    Fail,
}

impl From<&AuditEntry> for AuditResult {
    fn from(e: &AuditEntry) -> Self {
        AuditResult {
            check_name: format!("{}/{} — {}", e.namespace, e.resource, e.action),
            status: if e.success {
                AuditStatus::Pass
            } else {
                AuditStatus::Fail
            },
            details: e.details.clone(),
            remediation: e.error.clone(),
        }
    }
}

pub struct AuditReporter {
    client: S3Client,
    bucket: String,
    prefix: String,
}

impl AuditReporter {
    pub async fn new(bucket: String, prefix: String) -> Self {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let client = S3Client::new(&sdk_config);
        Self {
            client,
            bucket,
            prefix,
        }
    }

    pub async fn list(
        &self,
        limit: usize,
        resource_filter: Option<String>,
        actor_filter: Option<String>,
        json: bool,
    ) -> Result<()> {
        let objects = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&self.prefix)
            .send()
            .await
            .map_err(|e| Error::InternalError(format!("Failed to list audit logs: {e}")))?;

        let mut entries = Vec::new();

        if let Some(contents) = objects.contents {
            let mut sorted_contents = contents;
            sorted_contents.sort_by(|a, b| b.key().cmp(&a.key()));

            for obj in sorted_contents.iter().take(limit * 2) {
                if let Some(key) = obj.key() {
                    if !key.ends_with(".json") {
                        continue;
                    }

                    let data = self
                        .client
                        .get_object()
                        .bucket(&self.bucket)
                        .key(key)
                        .send()
                        .await
                        .map_err(|e| {
                            Error::InternalError(format!("Failed to fetch log {key}: {e}"))
                        })?;

                    let body = data.body.collect().await.map_err(|e| {
                        Error::InternalError(format!("Failed to read log {key}: {e}"))
                    })?;

                    if let Ok(entry) = serde_json::from_slice::<AuditEntry>(&body.into_bytes()) {
                        let matches_resource = resource_filter
                            .as_ref()
                            .is_none_or(|r| entry.resource.contains(r));
                        let matches_actor = actor_filter.as_ref().is_none_or(|a| entry.actor == *a);

                        if matches_resource && matches_actor {
                            entries.push(entry);
                        }
                    }

                    if entries.len() >= limit {
                        break;
                    }
                }
            }
        }

        if json {
            let report = AuditReport {
                timestamp: chrono::Utc::now().to_rfc3339(),
                cluster_name: std::env::var("STELLAR_CLUSTER_NAME")
                    .unwrap_or_else(|_| "unknown".to_string()),
                results: entries.iter().map(AuditResult::from).collect(),
            };
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            let mut table = Table::new();
            table.set_header(vec![
                "ID",
                "Timestamp",
                "Action",
                "Actor",
                "Resource",
                "Success",
            ]);
            for entry in entries {
                table.add_row(vec![
                    entry.id,
                    entry.timestamp.to_rfc3339(),
                    entry.action.to_string(),
                    entry.actor,
                    format!("{}/{}", entry.namespace, entry.resource),
                    entry.success.to_string(),
                ]);
            }
            println!("{table}");
        }

        Ok(())
    }

    pub async fn show(&self, id: &str, json: bool) -> Result<()> {
        let objects = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&self.prefix)
            .send()
            .await
            .map_err(|e| Error::InternalError(format!("Failed to search audit logs: {e}")))?;

        if let Some(contents) = objects.contents {
            for obj in contents {
                if let Some(key) = obj.key() {
                    if key.contains(id) && key.ends_with(".json") {
                        let data = self
                            .client
                            .get_object()
                            .bucket(&self.bucket)
                            .key(key)
                            .send()
                            .await
                            .map_err(|e| {
                                Error::InternalError(format!("Failed to fetch log {key}: {e}"))
                            })?;

                        let body = data.body.collect().await.map_err(|e| {
                            Error::InternalError(format!("Failed to read log {key}: {e}"))
                        })?;

                        let entry: AuditEntry = serde_json::from_slice(&body.into_bytes())
                            .map_err(|e| {
                                Error::InternalError(format!("Failed to parse log: {e}"))
                            })?;

                        if json {
                            let report = AuditReport {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                cluster_name: std::env::var("STELLAR_CLUSTER_NAME")
                                    .unwrap_or_else(|_| "unknown".to_string()),
                                results: vec![AuditResult::from(&entry)],
                            };
                            println!("{}", serde_json::to_string_pretty(&report).unwrap());
                        } else {
                            println!("Audit Entry Details:");
                            println!("--------------------");
                            println!("ID:        {}", entry.id);
                            println!("Timestamp: {}", entry.timestamp);
                            println!("Action:    {}", entry.action);
                            println!("Actor:     {}", entry.actor);
                            if let Some(meta) = &entry.actor_metadata {
                                println!(
                                    "Actor Meta: {}",
                                    serde_json::to_string_pretty(meta).unwrap()
                                );
                            }
                            println!("Resource:  {}/{}", entry.namespace, entry.resource);
                            println!("Success:   {}", entry.success);
                            if let Some(diff) = &entry.diff {
                                println!("\nChanges (Diff):");
                                println!("{}", serde_json::to_string_pretty(diff).unwrap());
                            }
                            if let Some(details) = &entry.details {
                                println!("\nDetails:");
                                println!("{details}");
                            }
                            if let Some(err) = &entry.error {
                                println!("\nError:");
                                println!("{err}");
                            }
                        }

                        return Ok(());
                    }
                }
            }
        }

        println!("Audit entry with ID {id} not found.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use stellar_k8s::controller::audit_log::{AdminAction, AuditEntry};

    fn make_entry(success: bool) -> AuditEntry {
        let mut e = AuditEntry::new(
            AdminAction::NodeCreate,
            "ci-bot",
            "my-validator",
            "stellar-system",
            Some("test details"),
        );
        e.success = success;
        if !success {
            e.error = Some("permission denied".to_string());
        }
        e
    }

    #[test]
    fn audit_report_serializes_and_deserializes() {
        let entries = vec![make_entry(true), make_entry(false)];
        let report = AuditReport {
            timestamp: Utc::now().to_rfc3339(),
            cluster_name: "test-cluster".to_string(),
            results: entries.iter().map(AuditResult::from).collect(),
        };

        let json_str = serde_json::to_string_pretty(&report).expect("serialization failed");

        // Must round-trip cleanly
        let parsed: AuditReport = serde_json::from_str(&json_str).expect("deserialization failed");

        assert_eq!(parsed.cluster_name, "test-cluster");
        assert_eq!(parsed.results.len(), 2);

        // First entry: PASS
        assert!(matches!(parsed.results[0].status, AuditStatus::Pass));
        assert_eq!(parsed.results[0].details.as_deref(), Some("test details"));
        assert!(parsed.results[0].remediation.is_none());

        // Second entry: FAIL with remediation
        assert!(matches!(parsed.results[1].status, AuditStatus::Fail));
        assert_eq!(
            parsed.results[1].remediation.as_deref(),
            Some("permission denied")
        );
    }

    #[test]
    fn audit_status_serializes_uppercase() {
        let pass = serde_json::to_string(&AuditStatus::Pass).unwrap();
        let fail = serde_json::to_string(&AuditStatus::Fail).unwrap();
        assert_eq!(pass, r#""PASS""#);
        assert_eq!(fail, r#""FAIL""#);
    }
}
