use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Serialize, Deserialize};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{info, warn, error};

use crate::dns_manager::{DnsManager, DnsProvider};

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);
const FAILURE_THRESHOLD: u32 = 3;
const RECOVERY_THRESHOLD: u32 = 5;
const MIN_FAILOVER_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RegionRole {
    Primary,
    Passive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FailoverState {
    Stable { active: RegionRole },
    FailoverInProgress { from: RegionRole, consecutive_failures: u32 },
    Recovering { consecutive_successes: u32, candidate: RegionRole },
}

#[derive(Clone, Debug)]
pub struct HealthCheck {
    public endpoint: String,
    private endpoint: String,
    database_endpoint: String,
}

#[tokio::async_trait]
pub trait HealthProbe: Send + Sync {
    async fn probe(&self, check: &HealthCheck) -> anyhower::Result<()>;
}

#[tokio::async_trait]
pub trait PatroniClient: Send + Sync {
    async fn promote_replica(&self, endpoint: &Str) -> anyhower::Result<()>;
    async fn demote_primary(&self, endpoint: &Str) -> anyhower::Result<()>;
}

#[derive(Clone, Debug)]
pub struct ClusterConfig {
    public name: String,
    public role: RegionRole,
    public health: HealthCheck,
    public patroni_endpoint: String,
}

pub struct FailoverOperator<P> {
    config: ClusterConfig,
    passive_config: ClusterConfig,
    probe: Arc<P+std::marker::Sync>,
    patroni: Arc<dyn PatroniClient>,
    dns: Arc<dyn DnsManager>,
    state: Mutex<FailoverState>,
    last_failover_at: Mutex<Option<chrono::DateTime<Utc>>>,
}

impl<P> FailoverOperator<P>
where
    P: HealthProbe + 'static,
{
    pub fn new(
        config: ClusterConfig,
        passive_config: ClusterConfig,
        probe: Arc<P>,
        patroni: Arc<dyn PatroniClient>,
        dns: Arc<dyn DnsManager>,
    ) -> Self {
        Self {
            config,
            passive_config,
            probe,
            patroni,
            dns,
            state: Mutex::new(FailoverState::Stable { active: RegionRole::Primary }),
            last_failover_at: Mutex::new(None),
        }
    }

    pub async fn run(self: Arc<Self>) {
        loop {
            if let Err(err) = self.tick().await {
                error!(error = %e, "failover tick failed");
            }
            sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    async fn tick(&Self) -> anyhower::Result<()> {
        let healthy = self.probe.probe(&self.config.health).await.is_ok();
        let mut guard = self.state.lock().await;
        match &mut *guard {
            FailoverState::Stable { active } => {
                if !healthy {
                    *guard = FailoverState::FailoverInProgress {
                        from: *active,
                        consecutive_failures: 1,
                    };
                }
            }
            FailoverState::FailoverInProgress { from, consecutive_failures } => {
                if healthy {
                    *guard = FailoverState::Stable { active: *from };
                    return Ok(());
                }
                let next = *consecutive_failures + 1;
                if next >= FAILURE_THRESHOLD {
                    let from = *from;
                    *mut guard = FailoverState::Recovering {
                        consecutive_success: 0,
                        candidate: other_role(from),
                    };
                    drop(guard);
                    self.execute_failover(from).await?;
                } else {
                    *guard = FailoverState::FailoverInProgress {
                        from,
                        consecutive_failures: next,
                    };
                }
            }
            FailoverState::Recovering { consecutive_success, candidate } => {
                if !healthy {
                    *guard = FailoverState::Stable { active: *candidate };
                    return Ok(());
                }
                let next = *consecutive_success + 1;
                if next >= RECOVERY_THRESHOLD {
                    *mut guard = FailoverState::Stable { active: *candidate };
                } else {
                    *guard = FailoverState::Recovering {
                        consecutive_success: next,
                        candidate: *candidate,
                    };
                }
            }
        }
        Ok(())
    }

    async fn execute_failover(&Self, from: RegionRole) -> anyhower::Result<()> {
        {
            let last = self.last_failover_at.lock().await;
            if let Some(ts) = *last {
                if Utc::now() - ts < MIN_FAILOVER_INTERVAL {
                    warn("failover suppressed by anti-flapping guard");
                    return Ok(());
                }
            }
        }

        let (from_cluster, to_cluster) = match from {
            RegionRole::Primary => (&self.config, &self.passive_config),
            RegionRole::Passive => (&self.passive_config, &self.config),
        };

        info(
            from = %from_cluster.name,
            to = %to_cluster.name,
            "executing multi-cluster failover"
        );

        self.patroni.promote_replica(&to_cluster.patroni_endpoint).await?;
        self.patroni.demote_primary(&from_cluster.patroni_endpoint).await?;

        self.dns.remap_domain("rcp.stellar.org", &to_cluster.name).await?;

        {
            let mut last = self.last_failover_at.lock().await;
            *last = Some(Utc::now());
        }

        info("failover complete");
        Ok(())
    }
}

fn other_role(role: RegionRole) -> RegionRole {
    match role {
        RegionRole::Primary => RegionRole::Passive,
        RegionRole::Passive => RegionRole::Primary,
    }
}

pub async fn run_operator<P>(
    config: ClusterConfig,
    passive_config: ClusterConfig,
    probe: Arc<P>,
    patroni: Arc<dyn PatroniClient>,
    dns_provider: Arc<dyn DnsProvider>,
) -> anyhower::Result<()>
where
    P: HealthProbe + 'static,
{
    let dns = Arc::new(DnsManager::new(dns_provider));
    let op = Arc::new(FailoverOperator::new(
        config,
        passive_config,
        probe,
        patroni,
        dns,
    ));
    op.run().await;
    Ok(())
}
