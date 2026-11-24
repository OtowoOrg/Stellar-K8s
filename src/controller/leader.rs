//! Leader election helpers backed by Kubernetes Leases.

use chrono::Utc;
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use kube::Client;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::Result;

const DEFAULT_LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;
const RENEW_INTERVAL: Duration = Duration::from_secs(10);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

static IS_LEADER: AtomicBool = AtomicBool::new(false);

/// Returns `true` if this process currently holds the leader lease.
pub fn is_leader() -> bool {
    IS_LEADER.load(Ordering::SeqCst)
}

/// Guard proving a one-off maintenance job owns its Kubernetes Lease.
#[derive(Debug)]
pub struct LeaseGuard {
    lease_name: String,
}

impl LeaseGuard {
    /// Acquire a short-lived Kubernetes Lease for exclusive maintenance work.
    pub async fn acquire(scope: impl Into<String>) -> Result<Self> {
        let scope = scope.into();
        let lease_name = format!("stellar-k8s-{scope}");
        let namespace = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "default".to_string());
        let identity = std::env::var("POD_NAME")
            .ok()
            .or_else(|| {
                hostname::get()
                    .ok()
                    .map(|host| host.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| format!("unknown-{}", std::process::id()));
        let client = Client::try_default().await?;
        let leases: Api<Lease> = Api::namespaced(client, &namespace);

        match try_acquire_or_renew(&leases, &lease_name, &namespace, &identity).await? {
            true => Ok(Self { lease_name }),
            false => Err(crate::Error::ConfigError(format!(
                "lease {lease_name} is held by another operator"
            ))),
        }
    }

    /// Name of the Kubernetes Lease held by this guard.
    pub fn lease_name(&self) -> &str {
        &self.lease_name
    }
}

/// Handle to the background leader-election task.
pub struct LeaderElectionHandle {
    task: JoinHandle<()>,
    leadership_changed: watch::Receiver<bool>,
}

impl LeaderElectionHandle {
    /// Starts a background lease renewal loop.
    pub fn start(
        client: Client,
        lease_name: Option<String>,
        lease_namespace: Option<String>,
        identity: Option<String>,
    ) -> Result<Self> {
        let name = lease_name
            .or_else(|| std::env::var("OPERATOR_LEASE_NAME").ok())
            .unwrap_or_else(|| DEFAULT_LEASE_NAME.to_string());
        let namespace = lease_namespace
            .or_else(|| std::env::var("POD_NAMESPACE").ok())
            .unwrap_or_else(|| "default".to_string());
        let holder = identity
            .or_else(|| std::env::var("POD_NAME").ok())
            .or_else(|| {
                hostname::get()
                    .ok()
                    .map(|host| host.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "unknown".to_string());

        let (tx, rx) = watch::channel(false);
        IS_LEADER.store(false, Ordering::SeqCst);

        let task = tokio::spawn(run_leader_election(client, name, namespace, holder, tx));

        Ok(Self {
            task,
            leadership_changed: rx,
        })
    }

    /// Returns `true` if this process currently holds the lease.
    pub fn is_leader(&self) -> bool {
        is_leader()
    }

    /// Waits until this process becomes the leader.
    pub async fn wait_until_leader(&self) {
        let mut rx = self.leadership_changed.clone();
        let _ = rx.wait_for(|value| *value).await;
    }

    /// Waits until this process loses leadership.
    pub async fn wait_until_lost(&self) {
        let mut rx = self.leadership_changed.clone();
        while *rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// Stops the background leader-election task.
    pub fn shutdown(self) {
        self.task.abort();
    }
}

async fn run_leader_election(
    client: Client,
    lease_name: String,
    namespace: String,
    identity: String,
    tx: watch::Sender<bool>,
) {
    let leases: Api<Lease> = Api::namespaced(client, &namespace);

    loop {
        match try_acquire_or_renew(&leases, &lease_name, &namespace, &identity).await {
            Ok(true) => {
                if !IS_LEADER.load(Ordering::Relaxed) {
                    info!("Acquired leadership: {}", lease_name);
                }
                IS_LEADER.store(true, Ordering::Relaxed);
                let _ = tx.send(true);
                tokio::time::sleep(RENEW_INTERVAL).await;
            }
            Ok(false) => {
                if IS_LEADER.load(Ordering::Relaxed) {
                    warn!("Lost leadership: {}", lease_name);
                }
                IS_LEADER.store(false, Ordering::Relaxed);
                let _ = tx.send(false);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(error) => {
                warn!("Leader election error: {:?}", error);
                IS_LEADER.store(false, Ordering::Relaxed);
                let _ = tx.send(false);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
        }
    }
}

async fn try_acquire_or_renew(
    leases: &Api<Lease>,
    lease_name: &str,
    namespace: &str,
    identity: &str,
) -> std::result::Result<bool, kube::Error> {
    let now = Utc::now();

    match leases.get(lease_name).await {
        Ok(existing) => {
            let spec = existing.spec.as_ref();
            let current_holder = spec.and_then(|s| s.holder_identity.as_deref());

            if current_holder == Some(identity) {
                renew_lease(leases, lease_name, identity, now).await?;
                return Ok(true);
            }

            if lease_expired(spec, now) {
                claim_lease(leases, lease_name, identity, now).await?;
                return Ok(true);
            }

            Ok(false)
        }
        Err(kube::Error::Api(error)) if error.code == 404 => {
            create_lease(leases, lease_name, namespace, identity, now).await?;
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

async fn renew_lease(
    leases: &Api<Lease>,
    lease_name: &str,
    identity: &str,
    now: chrono::DateTime<Utc>,
) -> std::result::Result<(), kube::Error> {
    let patch = serde_json::json!({
        "spec": {
            "holderIdentity": identity,
            "renewTime": MicroTime(now),
            "leaseDurationSeconds": LEASE_DURATION_SECS,
        }
    });
    leases
        .patch(lease_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

async fn claim_lease(
    leases: &Api<Lease>,
    lease_name: &str,
    identity: &str,
    now: chrono::DateTime<Utc>,
) -> std::result::Result<(), kube::Error> {
    let patch = serde_json::json!({
        "spec": {
            "holderIdentity": identity,
            "acquireTime": MicroTime(now),
            "renewTime": MicroTime(now),
            "leaseDurationSeconds": LEASE_DURATION_SECS,
        }
    });
    leases
        .patch(lease_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

async fn create_lease(
    leases: &Api<Lease>,
    lease_name: &str,
    namespace: &str,
    identity: &str,
    now: chrono::DateTime<Utc>,
) -> std::result::Result<(), kube::Error> {
    let lease = Lease {
        metadata: ObjectMeta {
            name: Some(lease_name.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(LeaseSpec {
            holder_identity: Some(identity.to_string()),
            acquire_time: Some(MicroTime(now)),
            renew_time: Some(MicroTime(now)),
            lease_duration_seconds: Some(LEASE_DURATION_SECS),
            ..Default::default()
        }),
    };
    leases.create(&PostParams::default(), &lease).await?;
    Ok(())
}

fn lease_expired(spec: Option<&LeaseSpec>, now: chrono::DateTime<Utc>) -> bool {
    spec.and_then(|s| s.renew_time.as_ref())
        .map(|renew| {
            let duration = spec
                .and_then(|s| s.lease_duration_seconds)
                .unwrap_or(LEASE_DURATION_SECS);
            let expiry = renew.0 + chrono::Duration::seconds(duration as i64);
            now > expiry
        })
        .unwrap_or(true)
}
