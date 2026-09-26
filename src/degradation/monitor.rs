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
//! Control-plane health monitor loop.
//!
//! Probes components, drives the [`DegradationTracker`], updates the shared
//! [`DegradationGate`], and publishes the `ControlPlaneHealth` status. All
//! state lives in memory, so detection continues while etcd is unavailable;
//! status writes are skipped at `Frozen` and the full status (including the
//! post-incident report) is published on the first round after recovery.

use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::admissionregistration::v1::{
    MutatingWebhookConfiguration, ValidatingWebhookConfiguration,
};
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::{Client, Resource};
use serde::de::DeserializeOwned;
use serde_json::json;
use tracing::{debug, info, warn};

use super::probes::{default_probes, run_probes};
use super::tracker::{DegradationTracker, RoundOutcome, TrackerConfig};
use super::DegradationGate;
use crate::crd::control_plane_health::{
    ComponentState, ControlPlaneHealth, ControlPlaneHealthSpec, WebhookConfigurationKind,
    WebhookConfigurationRef, WebhookMode,
};

/// Name of the cluster-scoped `ControlPlaneHealth` singleton.
pub const RESOURCE_NAME: &str = "cluster";
const FIELD_MANAGER: &str = "stellar-degradation-monitor";

pub struct ControlPlaneHealthMonitor {
    client: Client,
    gate: DegradationGate,
    is_leader: Arc<AtomicBool>,
}

impl ControlPlaneHealthMonitor {
    pub fn new(client: Client, gate: DegradationGate, is_leader: Arc<AtomicBool>) -> Self {
        Self {
            client,
            gate,
            is_leader,
        }
    }

    /// Runs forever. Every replica probes and updates its own gate; only the
    /// leader writes to the API.
    pub async fn run(self) {
        let api: Api<ControlPlaneHealth> = Api::all(self.client.clone());
        let mut spec = ControlPlaneHealthSpec::default();
        let mut generation = None;
        let mut probes = default_probes(&self.client, &spec);
        let mut tracker = DegradationTracker::new(TrackerConfig::from(&spec), Utc::now());
        info!("Control-plane health monitor started");

        loop {
            if let Some((loaded, gen)) = self.load_spec(&api).await {
                if loaded != spec {
                    info!(?loaded, "ControlPlaneHealth spec updated");
                    probes = default_probes(&self.client, &loaded);
                    tracker.set_config(TrackerConfig::from(&loaded));
                    spec = loaded;
                }
                generation = gen;
            }

            let timeout = Duration::from_secs(u64::from(spec.probe_timeout_seconds.max(1)));
            let results = run_probes(&probes, timeout).await;
            let outcome = tracker.record_round(&results, self.gate.suppressed_total(), Utc::now());
            self.gate.set_level(tracker.level());
            report(&outcome, &tracker);

            if self.is_leader.load(Ordering::Relaxed) && tracker.level().permitted().writes {
                if spec.webhook_mode == WebhookMode::Permissive {
                    if let Err(e) =
                        enforce_permissive(&self.client, &spec.webhook.configurations).await
                    {
                        warn!("Failed to enforce permissive webhook policy: {e}");
                    }
                }
                let status = tracker.status(generation);
                let patch = json!({
                    "apiVersion": ControlPlaneHealth::api_version(&()),
                    "kind": ControlPlaneHealth::kind(&()),
                    "status": status,
                });
                if let Err(e) = api
                    .patch_status(
                        RESOURCE_NAME,
                        &PatchParams::apply(FIELD_MANAGER).force(),
                        &Patch::Apply(&patch),
                    )
                    .await
                {
                    // State is kept in memory; the next round republishes it.
                    debug!("Deferring ControlPlaneHealth status publish: {e}");
                }
            }

            let interval = Duration::from_secs(u64::from(spec.probe_interval_seconds.max(1)));
            tokio::time::sleep(interval).await;
        }
    }

    /// Current spec, creating the default singleton if absent. `None` keeps
    /// the last known spec (e.g. while etcd is down).
    async fn load_spec(
        &self,
        api: &Api<ControlPlaneHealth>,
    ) -> Option<(ControlPlaneHealthSpec, Option<i64>)> {
        match api.get_opt(RESOURCE_NAME).await {
            Ok(Some(cr)) => Some((cr.spec, cr.metadata.generation)),
            Ok(None) => {
                if self.is_leader.load(Ordering::Relaxed) {
                    let cr =
                        ControlPlaneHealth::new(RESOURCE_NAME, ControlPlaneHealthSpec::default());
                    match api.create(&PostParams::default(), &cr).await {
                        Ok(_) => info!("Created default ControlPlaneHealth/{RESOURCE_NAME}"),
                        Err(e) => debug!("Could not create ControlPlaneHealth: {e}"),
                    }
                }
                None
            }
            Err(e) => {
                debug!("Using last known ControlPlaneHealth spec: {e}");
                None
            }
        }
    }
}

/// Logs and records metrics for one round.
fn report(outcome: &RoundOutcome, tracker: &DegradationTracker) {
    let status = tracker.status(None);
    for (component, from, to) in &outcome.component_changes {
        let message = status
            .components
            .iter()
            .find(|c| c.component == *component)
            .and_then(|c| c.message.clone())
            .unwrap_or_default();
        match to {
            ComponentState::Unhealthy => {
                warn!(?component, ?from, %message, "Control-plane component unhealthy")
            }
            ComponentState::Healthy => info!(?component, ?from, "Control-plane component healthy"),
            ComponentState::Unknown => {}
        }
    }

    if let Some(t) = &outcome.transition {
        if t.to > t.from {
            warn!(from = ?t.from, to = ?t.to, reason = %t.reason, "Control-plane degradation level raised");
        } else {
            info!(from = ?t.from, to = ?t.to, reason = %t.reason, "Control-plane degradation level lowered");
        }
        #[cfg(feature = "metrics")]
        crate::controller::metrics::inc_control_plane_degradation_transition(
            &format!("{:?}", t.from),
            &format!("{:?}", t.to),
        );
    }

    if let Some(r) = &outcome.closed_incident {
        info!(
            incident = %r.id,
            duration_seconds = r.duration_seconds.unwrap_or_default(),
            peak_level = ?r.peak_level,
            components = ?r.components,
            suppressed_actions = r.suppressed_actions,
            report = %serde_json::to_string(r).unwrap_or_default(),
            "Control-plane incident closed; recovered automatically"
        );
    }

    #[cfg(feature = "metrics")]
    {
        use crate::controller::metrics;
        metrics::set_control_plane_degradation_level(i64::from(status.level.as_index()));
        for c in &status.components {
            let value = match c.state {
                ComponentState::Healthy => 1,
                ComponentState::Unhealthy => 0,
                ComponentState::Unknown => -1,
            };
            metrics::set_control_plane_component_state(&format!("{:?}", c.component), value);
        }
    }
}

/// Keeps the referenced webhooks at `failurePolicy: Ignore` so an outage of
/// the webhook layer can never block admission.
async fn enforce_permissive(
    client: &Client,
    refs: &[WebhookConfigurationRef],
) -> Result<(), kube::Error> {
    for r in refs {
        match r.kind {
            WebhookConfigurationKind::Validating => {
                let api: Api<ValidatingWebhookConfiguration> = Api::all(client.clone());
                fail_open(&api, &r.name, |c| {
                    not_ignoring(
                        c.webhooks
                            .iter()
                            .flatten()
                            .map(|w| (&w.name, &w.failure_policy)),
                    )
                })
                .await?
            }
            WebhookConfigurationKind::Mutating => {
                let api: Api<MutatingWebhookConfiguration> = Api::all(client.clone());
                fail_open(&api, &r.name, |c| {
                    not_ignoring(
                        c.webhooks
                            .iter()
                            .flatten()
                            .map(|w| (&w.name, &w.failure_policy)),
                    )
                })
                .await?
            }
        }
    }
    Ok(())
}

fn not_ignoring<'a>(
    webhooks: impl Iterator<Item = (&'a String, &'a Option<String>)>,
) -> Vec<String> {
    webhooks
        .filter(|(_, policy)| policy.as_deref() != Some("Ignore"))
        .map(|(name, _)| name.clone())
        .collect()
}

async fn fail_open<K>(
    api: &Api<K>,
    name: &str,
    needing: impl Fn(&K) -> Vec<String>,
) -> Result<(), kube::Error>
where
    K: Resource + Clone + DeserializeOwned + Debug,
{
    let Some(config) = api.get_opt(name).await? else {
        return Ok(());
    };
    let webhooks = needing(&config);
    if webhooks.is_empty() {
        return Ok(());
    }
    // `webhooks` merges by `name` under strategic merge patch.
    let patch = json!({
        "webhooks": webhooks
            .iter()
            .map(|w| json!({ "name": w, "failurePolicy": "Ignore" }))
            .collect::<Vec<_>>()
    });
    let params = PatchParams {
        field_manager: Some(FIELD_MANAGER.to_string()),
        ..Default::default()
    };
    api.patch(name, &params, &Patch::Strategic(patch)).await?;
    warn!(
        configuration = name,
        ?webhooks,
        "Set failurePolicy=Ignore (webhookMode=Permissive)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_webhooks_not_already_ignoring() {
        let hooks = [
            ("a".to_string(), Some("Fail".to_string())),
            ("b".to_string(), Some("Ignore".to_string())),
            ("c".to_string(), None),
        ];
        let names = not_ignoring(hooks.iter().map(|(n, p)| (n, p)));
        assert_eq!(names, ["a", "c"]);
    }
}
