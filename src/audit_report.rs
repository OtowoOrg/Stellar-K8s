use aws_sdk_s3::Client as S3Client;
use comfy_table::Table;
use serde_json::Value;
use tracing::error;

use stellar_k8s::controller::audit_log::AuditEntry;
use stellar_k8s::error::{Error, Result};

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
            // Sort by key descending to get newest logs first
            let mut sorted_contents = contents;
            sorted_contents.sort_by(|a, b| b.key().cmp(&a.key()));

            for obj in sorted_contents.iter().take(limit * 2) {
                // Over-fetch to allow filtering
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
                            .map_or(true, |r| entry.resource.contains(r));
                        let matches_actor =
                            actor_filter.as_ref().map_or(true, |a| entry.actor == *a);

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
        Ok(())
    }

    pub async fn show(&self, id: &str) -> Result<()> {
        // Since logs are partitioned by date, we need to search or use a global index.
        // For simplicity, we'll list all objects and find the one with the ID in its key.
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

                        return Ok(());
                    }
                }
            }
        }

        println!("Audit entry with ID {id} not found.");
        Ok(())
    }
}
