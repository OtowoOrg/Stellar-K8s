use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Event;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use kube::api::{Api, DeleteParams, PostParams};
use kube::runtime::wait::{await_condition, Condition};
use kube::Client;
use serde_json::json;
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
use tracing::{error, info, warn};

#[derive(Error, Debug)]
pub enum MigrationJobError {
    #[error("failed to create migration job: {0}")]
    CreationFailed(kube::Error),
    #[error("failed to delete existing migration job: {0}")]
    DeletionFailed(kube::Error),
    #[error("migration job failed: {0}")]
    JobFailed(String),
    #[error("migration job timed out after {0:?}")]
    Timeout(Duration),
    #[error("failed to create diagnostic event: {0}")]
    EventCreationFailed(kube::Error),
    #[error("failed to serialize job object: {0}")]
    SerializationError(serde_json::Error),
}

pub struct MigrationJob {
    client: Client,
    job: Job,
}

impl MigrationJob {
    pub fn new(client: Client, job: Job) -> Self {
        Self { client, job }
    }

    pub async fn run(&self, timeout: Duration) -> Result<(), MigrationJobError> {
        let namespace = self.job.metadata.namespace
            .as_deref()
            .unwrap_or("default")
            .to_string();
        let name = self.job.metadata.name
            .as_deref()
            .ok_or_else(MigrationJobError::JobFailed("job name is required".into()))?;
        let renamed = name.to_string();
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &namespace);

        // Clean up any previous job with the same name.
        match jobs.get(&renamed).await {
            Ok(_) => {
                warn!("Migration job {}/{} already exists, deleting", namespace, name);
                jobs.delete(&renamed, &DeleteParams::default()).await
                    .map_err(MigrationJobError::DeletionFailed)?;
            }
            Err(kube::Error::NotFound) => {}
            Err(e) => return Err(MigrationJobError::CreationFailed(e)),
        }

        // Create the migration job.
        info!("Creating migration job {}/{}", namespace, name);
        jobs.create(&self.job, &PostParams::default())
            .await
            .map_err(MigrationJobError::CreationFailed)?;

        // Wait for the job to reach a terminal state (Complete or Failed).
        let condition = {
            | data: Option<&Job>| -> bool {
                if let Some(job) = data {
                    if let Some(status) = &job.status {
                        let conditions = status.conditions.as_deref().unwrap_or(&[]);
                        conditions
                            .iter()
                            .any(|c| {
                                c.type_.as_deref() == Some("Complete")
                                    && c.status.as_deref() == Some("True")
                            })
                            || conditions.iter().any(|c| {
                                c.type_.as_deref() == Some("Failed")
                                    && c.status.as_deref() == Some("True")
                            })
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        };

        match timeout(timeout, await_condition(jobs.clone(), &renamed, condition)).await {
            Ok(Ok(Some(job))) => {
                if job_succeeded(&job) {
                    info!("Migration job {}/{} succeeded", namespace, name);
                    Ok(())
                } else {
                    error!("Migration job {}/{} failed", namespace, name);
                    self.raise_event("MigrationFailed", format!("Migration job {}/{} failed", namespace, name)).await?;
                    Err(MigrationJobError::JobFailed(format!("{}/{}", namespace, name)))
                }
            },
            Ok(Ok(None)) => {
                error!("Migration job {}/{} was removed while waiting", namespace, name);
                Err(MigrationJobError::JobFailed(format!("job {}/{} disappeared", namespace, name)))
            },
            Ok(Err(e)) => {
                error!("Error watching migration job {}/{}: {:}", namespace, name, e);
                Err(MigrationJobError::JobFailed(format!("watch error: {:}", e)))
            },
            Err(elapsed) => {
                warn!("Migration job {}/{} timed out after {?}", namespace, name, elapsed);
                Err(MigrationJobError::Timeout(elapsed))
            }
        }
    }

    async fn raise_event(&self, reason: &str, message: String) -> Result<(), MigrationJobError> {
        let namespace = self.job.metadata.namespace
            .as_deref()
            .unwrap_or("default")
            .to_string();
        let name = self.job.metadata.name
            .as_deref()
            .unwrap_or("migration-job")
            .to_string();
        let now = chrono::Utc::now();
        let event_name = format!("{}.fail.{}", name, now.timestamp_millis());
        let events : Api<Event> = Api::namespaced(self.client.clone(), &namespace);
        let event = Event {
            metadata: ObjectMeta {
                name: Some(event_name),
                namespace: Some(namespace.clone()),
                ..Default::default()
            },
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                kind: Some("Job".to_string()),
                namespace: Some(namespace.clone()),
                name: Some(name.clone()),
                api_version: Some("batch/v1".to_string()),
                ..Default::default()
            },
            reason: Some(reason.to_string()),
            message: Some(message),
            first_timestamp: Some(Time(now)),
            last_timestamp: Some(Time(now)),
            count: Some(1),
            type_: Some("Warning".to_string()),
            source: Some(k8s_openapi::api::core::v1::EventSource {
                component: Some("horizon-migration-gate".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        events.create(&event, &PostParams::default()).await
            .map_err(MigrationJobError::EventCreationFailed)?;
        Ok(())
    }
}

fn job_succeeded(job: &Job) -> bool {
    if let Some(status) = &job.status {
        status
            .conditions
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|c| {
                c.type_.as_deref() == Some("Complete") && c.status.as_deref() == Some("True")
            })
    } else {
        false
    }
}
