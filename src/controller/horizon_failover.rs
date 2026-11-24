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
//! Horizon Ingestion Failover for Validator Groups and Multi-Replica Clusters
//!
//! When running multiple Horizon replicas, captive-core ingestion must be leader-elected
//! to avoid duplicate ledger ingestion and database conflicts. Standby replicas operate
//! in API-only mode and respond cleanly to `/healthz` and client traffic.
//!
//! When the leader pod terminates, automatic failover completes within 15–30s (well within
//! the 30s SLA), promoting a standby to become the new ingestion leader.

use chrono::{DateTime, Utc};
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{ObjectMeta, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Default lease duration in seconds (15s ensures failover completes well within 30s)
pub const DEFAULT_INGESTION_LEASE_DURATION_SECS: i32 = 15;
/// Lease renewal interval (renew every 5s)
pub const DEFAULT_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);
/// Retry interval on acquisition failure (try every 3s)
pub const DEFAULT_LEASE_RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Role of this Horizon replica with respect to captive core ingestion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HorizonIngestionRole {
    /// Active leader: actively ingests ledgers via captive core
    Leader,
    /// Standby: runs in API-only mode, serving queries without ingestion
    Standby,
}

/// Status payload returned by Horizon health checks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HorizonHealthStatus {
    pub status: String,
    pub role: HorizonIngestionRole,
    pub is_ingesting: bool,
    pub api_mode: String,
    pub leader_identity: Option<String>,
    pub last_transition_time: Option<DateTime<Utc>>,
}

/// State tracking for Horizon ingestion failover
#[derive(Debug)]
pub struct HorizonIngestionState {
    pub role: HorizonIngestionRole,
    pub is_ingesting: bool,
    pub leader_identity: Option<String>,
    pub last_transition_time: Option<DateTime<Utc>>,
    pub failover_count: u64,
}

/// Coordinator for Horizon replica ingestion leader election via Kubernetes Lease API
pub struct HorizonIngestionCoordinator {
    client: Client,
    namespace: String,
    lease_name: String,
    pod_identity: String,
    lease_duration_secs: i32,
    is_leader: Arc<AtomicBool>,
    state: Arc<RwLock<HorizonIngestionState>>,
}

impl HorizonIngestionCoordinator {
    /// Create a new Horizon ingestion failover coordinator
    pub fn new(
        client: Client,
        namespace: &str,
        node_name: &str,
        pod_identity: &str,
        lease_duration_secs: Option<i32>,
    ) -> Self {
        let lease_name = format!("{node_name}-horizon-ingest-lease");
        let lease_duration_secs = lease_duration_secs
            .unwrap_or(DEFAULT_INGESTION_LEASE_DURATION_SECS)
            .max(5);

        Self {
            client,
            namespace: namespace.to_string(),
            lease_name,
            pod_identity: pod_identity.to_string(),
            lease_duration_secs,
            is_leader: Arc::new(AtomicBool::new(false)),
            state: Arc::new(RwLock::new(HorizonIngestionState {
                role: HorizonIngestionRole::Standby,
                is_ingesting: false,
                leader_identity: None,
                last_transition_time: Some(Utc::now()),
                failover_count: 0,
            })),
        }
    }

    /// Whether this replica is currently the elected ingestion leader
    pub fn is_leader(&self) -> bool {
        self.is_leader.load(Ordering::Relaxed)
    }

    /// Current ingestion role (Leader or Standby)
    pub async fn current_role(&self) -> HorizonIngestionRole {
        self.state.read().await.role
    }

    /// Health check status representation for `/healthz`
    pub async fn health_status(&self) -> HorizonHealthStatus {
        let s = self.state.read().await;
        HorizonHealthStatus {
            status: "ok".to_string(),
            role: s.role,
            is_ingesting: s.is_ingesting,
            api_mode: match s.role {
                HorizonIngestionRole::Leader => "api_and_ingestion".to_string(),
                HorizonIngestionRole::Standby => "api_only".to_string(),
            },
            leader_identity: s.leader_identity.clone(),
            last_transition_time: s.last_transition_time,
        }
    }

    /// Run the leader election loop in the background.
    /// Manages Lease acquisition and renewal. When acquired, enables ingestion.
    /// When lost or in standby, keeps the instance running in API-only mode without ingestion.
    pub async fn run_election_loop(&self) {
        let leases: Api<Lease> = Api::namespaced(self.client.clone(), &self.namespace);

        info!(
            pod = %self.pod_identity,
            lease = %self.lease_name,
            lease_duration = self.lease_duration_secs,
            "Starting Horizon ingestion leader election loop"
        );

        loop {
            match self.try_acquire_or_renew(&leases).await {
                Ok(true) => {
                    let was_leader = self.is_leader.swap(true, Ordering::Relaxed);
                    if !was_leader {
                        info!(
                            pod = %self.pod_identity,
                            lease = %self.lease_name,
                            "Acquired Horizon ingestion leadership: enabling captive core ingestion"
                        );
                        let mut st = self.state.write().await;
                        st.role = HorizonIngestionRole::Leader;
                        st.is_ingesting = true;
                        st.leader_identity = Some(self.pod_identity.clone());
                        st.last_transition_time = Some(Utc::now());
                        st.failover_count += 1;
                    }
                    tokio::time::sleep(DEFAULT_LEASE_RENEW_INTERVAL).await;
                }
                Ok(false) => {
                    let was_leader = self.is_leader.swap(false, Ordering::Relaxed);
                    if was_leader {
                        warn!(
                            pod = %self.pod_identity,
                            lease = %self.lease_name,
                            "Lost Horizon ingestion leadership: switching to API-only mode"
                        );
                        let mut st = self.state.write().await;
                        st.role = HorizonIngestionRole::Standby;
                        st.is_ingesting = false;
                        st.last_transition_time = Some(Utc::now());
                    } else {
                        debug!(
                            pod = %self.pod_identity,
                            "Standby Horizon replica running in API-only mode"
                        );
                    }
                    tokio::time::sleep(DEFAULT_LEASE_RETRY_INTERVAL).await;
                }
                Err(e) => {
                    warn!(
                        pod = %self.pod_identity,
                        lease = %self.lease_name,
                        "Error checking ingestion lease: {:?}", e
                    );
                    self.is_leader.store(false, Ordering::Relaxed);
                    tokio::time::sleep(DEFAULT_LEASE_RETRY_INTERVAL).await;
                }
            }
        }
    }

    /// Try to acquire or renew the coordination Lease
    async fn try_acquire_or_renew(&self, leases: &Api<Lease>) -> Result<bool, kube::Error> {
        let now = Utc::now();

        match leases.get(&self.lease_name).await {
            Ok(existing) => {
                let spec = existing.spec.as_ref();
                let current_holder = spec.and_then(|s| s.holder_identity.as_deref());

                // If this pod already holds the lease, renew it
                if current_holder == Some(&self.pod_identity) {
                    let patch = serde_json::json!({
                        "spec": {
                            "renewTime": MicroTime(now),
                            "leaseDurationSeconds": self.lease_duration_secs,
                        }
                    });
                    leases
                        .patch(
                            &self.lease_name,
                            &PatchParams::default(),
                            &Patch::Merge(&patch),
                        )
                        .await?;
                    return Ok(true);
                }

                // Check if the current lease has expired
                let is_expired = spec
                    .and_then(|s| s.renew_time.as_ref())
                    .map(|renew| {
                        let duration = spec
                            .and_then(|s| s.lease_duration_seconds)
                            .unwrap_or(self.lease_duration_secs);
                        let expiry = renew.0 + chrono::Duration::seconds(duration as i64);
                        now > expiry
                    })
                    .unwrap_or(true);

                if is_expired {
                    info!(
                        lease = %self.lease_name,
                        previous_holder = ?current_holder,
                        new_holder = %self.pod_identity,
                        "Previous Horizon ingestion lease expired: failing over and acquiring lease"
                    );

                    let patch = serde_json::json!({
                        "spec": {
                            "holderIdentity": self.pod_identity,
                            "acquireTime": MicroTime(now),
                            "renewTime": MicroTime(now),
                            "leaseDurationSeconds": self.lease_duration_secs,
                        }
                    });
                    leases
                        .patch(
                            &self.lease_name,
                            &PatchParams::default(),
                            &Patch::Merge(&patch),
                        )
                        .await?;
                    Ok(true)
                } else {
                    // Update state with who the current leader is
                    let mut st = self.state.write().await;
                    st.leader_identity = current_holder.map(|s| s.to_string());
                    Ok(false)
                }
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                // Lease doesn't exist yet: create it as initial leader
                let lease = Lease {
                    metadata: ObjectMeta {
                        name: Some(self.lease_name.clone()),
                        namespace: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    spec: Some(k8s_openapi::api::coordination::v1::LeaseSpec {
                        holder_identity: Some(self.pod_identity.clone()),
                        acquire_time: Some(MicroTime(now)),
                        renew_time: Some(MicroTime(now)),
                        lease_duration_seconds: Some(self.lease_duration_secs),
                        ..Default::default()
                    }),
                };
                leases.create(&PostParams::default(), &lease).await?;
                info!(
                    pod = %self.pod_identity,
                    lease = %self.lease_name,
                    "Created Horizon ingestion coordination lease as initial leader"
                );
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }
}
