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
//! Independent control-plane component probes.
//!
//! Each probe exercises exactly one component and runs concurrently with the
//! others on its own timeout, so one hung dependency cannot mask another.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use k8s_openapi::api::coordination::v1::Lease;
use kube::{Api, Client};
use tokio::net::{lookup_host, TcpStream};

use super::tracker::ProbeOutcome;
use crate::crd::control_plane_health::{ControlPlaneComponent, ControlPlaneHealthSpec};

#[async_trait]
pub trait ComponentProbe: Send + Sync {
    fn component(&self) -> ControlPlaneComponent;
    async fn probe(&self) -> ProbeOutcome;
}

/// Runs all probes concurrently; a probe exceeding `timeout` is unhealthy.
pub async fn run_probes(
    probes: &[Box<dyn ComponentProbe>],
    timeout: Duration,
) -> Vec<(ControlPlaneComponent, ProbeOutcome)> {
    futures::future::join_all(probes.iter().map(|p| async move {
        let outcome = tokio::time::timeout(timeout, p.probe())
            .await
            .unwrap_or_else(|_| {
                ProbeOutcome::Unhealthy(format!("probe timed out after {}s", timeout.as_secs()))
            });
        (p.component(), outcome)
    }))
    .await
}

/// Builds the standard probe set for `spec`.
pub fn default_probes(
    client: &Client,
    spec: &ControlPlaneHealthSpec,
) -> Vec<Box<dyn ComponentProbe>> {
    vec![
        Box::new(EtcdProbe {
            client: client.clone(),
        }),
        Box::new(DnsProbe {
            hostname: spec.dns.hostname.clone(),
        }),
        Box::new(WebhookProbe::new(
            spec.webhook.host.clone(),
            spec.webhook.port,
        )),
        Box::new(SchedulerProbe {
            client: client.clone(),
            namespace: spec.scheduler.lease_namespace.clone(),
            name: spec.scheduler.lease_name.clone(),
        }),
    ]
}

/// etcd health as reported by the API server's `/readyz/etcd` check.
///
/// An unreachable API server is treated as unhealthy: from the operator's
/// perspective the storage path is gone either way.
pub struct EtcdProbe {
    pub client: Client,
}

#[async_trait]
impl ComponentProbe for EtcdProbe {
    fn component(&self) -> ControlPlaneComponent {
        ControlPlaneComponent::Etcd
    }

    async fn probe(&self) -> ProbeOutcome {
        let request = match http::Request::get("/readyz/etcd").body(Vec::new()) {
            Ok(r) => r,
            Err(e) => return ProbeOutcome::Inconclusive(format!("building request: {e}")),
        };
        match self.client.request_text(request).await {
            Ok(body) if body.trim() == "ok" => ProbeOutcome::Healthy,
            Ok(body) => ProbeOutcome::Unhealthy(format!("/readyz/etcd: {}", body.trim())),
            Err(kube::Error::Api(e)) if e.code == 401 || e.code == 403 => {
                ProbeOutcome::Inconclusive(format!("/readyz/etcd not permitted: {}", e.message))
            }
            Err(e) => ProbeOutcome::Unhealthy(format!("/readyz/etcd: {e}")),
        }
    }
}

/// Resolves a well-known name through cluster DNS.
pub struct DnsProbe {
    pub hostname: String,
}

#[async_trait]
impl ComponentProbe for DnsProbe {
    fn component(&self) -> ControlPlaneComponent {
        ControlPlaneComponent::Dns
    }

    async fn probe(&self) -> ProbeOutcome {
        match lookup_host((self.hostname.as_str(), 0)).await {
            Ok(mut addrs) => match addrs.next() {
                Some(_) => ProbeOutcome::Healthy,
                None => {
                    ProbeOutcome::Unhealthy(format!("{} resolved to no addresses", self.hostname))
                }
            },
            Err(e) => ProbeOutcome::Unhealthy(format!("resolving {}: {e}", self.hostname)),
        }
    }
}

/// TCP reachability of the admission webhook service.
///
/// The last resolved addresses are cached so a DNS outage is not
/// misreported as a webhook outage.
pub struct WebhookProbe {
    host: String,
    port: u16,
    cached: Mutex<Vec<SocketAddr>>,
}

impl WebhookProbe {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            cached: Mutex::new(Vec::new()),
        }
    }

    async fn addresses(&self) -> Option<Vec<SocketAddr>> {
        if let Ok(addrs) = lookup_host((self.host.as_str(), self.port)).await {
            let addrs: Vec<SocketAddr> = addrs.collect();
            if !addrs.is_empty() {
                *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = addrs.clone();
                return Some(addrs);
            }
        }
        let cached = self
            .cached
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        (!cached.is_empty()).then_some(cached)
    }
}

#[async_trait]
impl ComponentProbe for WebhookProbe {
    fn component(&self) -> ControlPlaneComponent {
        ControlPlaneComponent::Webhook
    }

    async fn probe(&self) -> ProbeOutcome {
        let Some(addrs) = self.addresses().await else {
            return ProbeOutcome::Inconclusive(format!(
                "cannot resolve {} and no cached address",
                self.host
            ));
        };
        let mut last_err = None;
        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(_) => return ProbeOutcome::Healthy,
                Err(e) => last_err = Some(format!("{addr}: {e}")),
            }
        }
        ProbeOutcome::Unhealthy(format!(
            "webhook {}:{} unreachable ({})",
            self.host,
            self.port,
            last_err.unwrap_or_default()
        ))
    }
}

/// Scheduler liveness from the freshness of its leader-election lease.
///
/// Clusters that do not expose the lease (most managed offerings) yield
/// inconclusive results, which never degrade the level.
pub struct SchedulerProbe {
    pub client: Client,
    pub namespace: String,
    pub name: String,
}

#[async_trait]
impl ComponentProbe for SchedulerProbe {
    fn component(&self) -> ControlPlaneComponent {
        ControlPlaneComponent::Scheduler
    }

    async fn probe(&self) -> ProbeOutcome {
        let api: Api<Lease> = Api::namespaced(self.client.clone(), &self.namespace);
        match api.get_opt(&self.name).await {
            Ok(Some(lease)) => lease_outcome(&lease, Utc::now()),
            Ok(None) => ProbeOutcome::Inconclusive(format!(
                "lease {}/{} not found",
                self.namespace, self.name
            )),
            Err(e) => ProbeOutcome::Inconclusive(format!("reading scheduler lease: {e}")),
        }
    }
}

/// Healthy while the lease was renewed within two lease durations.
fn lease_outcome(lease: &Lease, now: chrono::DateTime<Utc>) -> ProbeOutcome {
    let Some(spec) = lease.spec.as_ref() else {
        return ProbeOutcome::Inconclusive("scheduler lease has no spec".into());
    };
    let Some(renewed) = spec.renew_time.as_ref().map(|t| t.0) else {
        return ProbeOutcome::Unhealthy("scheduler lease never renewed".into());
    };
    let duration = i64::from(spec.lease_duration_seconds.unwrap_or(15).max(1));
    let age = (now - renewed).num_seconds();
    if age <= 2 * duration {
        ProbeOutcome::Healthy
    } else {
        ProbeOutcome::Unhealthy(format!(
            "scheduler lease last renewed {age}s ago (lease duration {duration}s)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::coordination::v1::LeaseSpec;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
    use std::net::TcpListener;

    fn lease(renewed_secs_ago: Option<i64>) -> Lease {
        let now = Utc::now();
        Lease {
            spec: Some(LeaseSpec {
                renew_time: renewed_secs_ago.map(|s| MicroTime(now - chrono::Duration::seconds(s))),
                lease_duration_seconds: Some(15),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn scheduler_lease_freshness() {
        assert_eq!(
            lease_outcome(&lease(Some(5)), Utc::now()),
            ProbeOutcome::Healthy
        );
        assert!(matches!(
            lease_outcome(&lease(Some(120)), Utc::now()),
            ProbeOutcome::Unhealthy(_)
        ));
        assert!(matches!(
            lease_outcome(&lease(None), Utc::now()),
            ProbeOutcome::Unhealthy(_)
        ));
        assert!(matches!(
            lease_outcome(&Lease::default(), Utc::now()),
            ProbeOutcome::Inconclusive(_)
        ));
    }

    #[tokio::test]
    async fn webhook_probe_detects_reachability_and_survives_dns_loss() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let probe = WebhookProbe::new("localhost".into(), port);
        assert_eq!(probe.probe().await, ProbeOutcome::Healthy);

        // Simulate DNS loss: the host no longer resolves, cached address is used.
        let probe = WebhookProbe {
            host: "stellar-webhook.invalid".into(),
            port,
            cached: Mutex::new(vec![listener.local_addr().unwrap()]),
        };
        assert_eq!(probe.probe().await, ProbeOutcome::Healthy);

        drop(listener);
        assert!(matches!(probe.probe().await, ProbeOutcome::Unhealthy(_)));

        let fresh = WebhookProbe::new("stellar-webhook.invalid".into(), port);
        assert!(matches!(fresh.probe().await, ProbeOutcome::Inconclusive(_)));
    }

    #[tokio::test]
    async fn dns_probe_reports_resolution_failures() {
        let ok = DnsProbe {
            hostname: "localhost".into(),
        };
        assert_eq!(ok.probe().await, ProbeOutcome::Healthy);
        let bad = DnsProbe {
            hostname: "does-not-exist.invalid".into(),
        };
        assert!(matches!(bad.probe().await, ProbeOutcome::Unhealthy(_)));
    }

    struct Hangs;
    #[async_trait]
    impl ComponentProbe for Hangs {
        fn component(&self) -> ControlPlaneComponent {
            ControlPlaneComponent::Etcd
        }
        async fn probe(&self) -> ProbeOutcome {
            std::future::pending().await
        }
    }

    struct Ok_(ControlPlaneComponent);
    #[async_trait]
    impl ComponentProbe for Ok_ {
        fn component(&self) -> ControlPlaneComponent {
            self.0
        }
        async fn probe(&self) -> ProbeOutcome {
            ProbeOutcome::Healthy
        }
    }

    #[tokio::test]
    async fn a_hung_probe_times_out_without_blocking_others() {
        let probes: Vec<Box<dyn ComponentProbe>> =
            vec![Box::new(Hangs), Box::new(Ok_(ControlPlaneComponent::Dns))];
        let results = run_probes(&probes, Duration::from_millis(50)).await;
        assert!(matches!(results[0].1, ProbeOutcome::Unhealthy(_)));
        assert_eq!(
            results[1],
            (ControlPlaneComponent::Dns, ProbeOutcome::Healthy)
        );
    }
}
