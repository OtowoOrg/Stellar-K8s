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
//! ServiceOwnershipRegistry reconciliation (epic #1522).
//!
//! Every cycle the registry is re-derived from live workload metadata rather
//! than trusted from a one-time import. Ownership is resolved per workload in
//! precedence order:
//!
//! 1. the owner label (`spec.ownerLabel`)
//! 2. the deploy-pipeline annotation (`spec.deployAnnotation`)
//! 3. CODEOWNERS, matched on the workload's source-path annotation
//!    (last matching rule wins, as on GitHub)
//!
//! Workloads with no owner are listed in `status.unowned`; owners that are
//! unknown teams or whose rotation is not live are listed in `status.stale`.
//! Both are pushed to Alertmanager in the same cycle. Ownership changes are
//! appended to `status.history`. [`attribute_alert`] and
//! [`alertmanager_routes`] make alert routing use the registry's attribution.

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::{Client, Resource, ResourceExt};
use std::collections::{BTreeMap, HashMap};
use tracing::{info, warn};

use crate::crd::service_ownership::{
    OwnershipChange, OwnershipEntry, OwnershipSource, ServiceOwnershipRegistry,
    ServiceOwnershipRegistrySpec, ServiceOwnershipRegistryStatus, StaleOwnership, TeamRotation,
    WorkloadRef,
};
use crate::crd::types::Condition;
use crate::error::Result;

/// Metadata of a running workload used for attribution.
#[derive(Debug, Clone, Default)]
pub struct WorkloadMeta {
    pub workload: WorkloadRef,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
}

/// Does a CODEOWNERS `pattern` match repository `path` (no leading slash)?
/// Supports `*`, `**`, anchored (`/x`) and directory (`x/`) patterns.
pub fn codeowners_match(pattern: &str, path: &str) -> bool {
    let path = path.trim_start_matches('/');
    let anchored = pattern.starts_with('/') || pattern.trim_end_matches('/').contains('/');
    let mut pat = pattern.trim_start_matches('/').to_string();
    if pat.ends_with('/') {
        pat.push_str("**");
    }
    if anchored {
        return glob(&pat, path) || glob(&format!("{pat}/**"), path);
    }
    // Unanchored: matches at any depth.
    let segments: Vec<&str> = path.split('/').collect();
    (0..segments.len()).any(|i| {
        let sub = segments[i..].join("/");
        glob(&pat, &sub) || glob(&format!("{pat}/**"), &sub)
    })
}

fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p {
            [] => t.is_empty(),
            [b'*', b'*', rest @ ..] => {
                let rest = rest.strip_prefix(b"/").unwrap_or(rest);
                (0..=t.len()).any(|i| go(rest, &t[i..]))
            }
            [b'*', rest @ ..] => (0..=t.len())
                .take_while(|&i| i == 0 || t[i - 1] != b'/')
                .any(|i| go(rest, &t[i..])),
            [c, rest @ ..] => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

/// Owners of `path` per CODEOWNERS contents (last matching rule wins).
pub fn codeowners_for<'a>(codeowners: &'a str, path: &str) -> Vec<&'a str> {
    let mut owners = Vec::new();
    for line in codeowners.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };
        if codeowners_match(pattern, path) {
            owners = parts.collect();
        }
    }
    owners
}

fn resolve(
    spec: &ServiceOwnershipRegistrySpec,
    w: &WorkloadMeta,
) -> Option<(String, OwnershipSource)> {
    if let Some(t) = w.labels.get(&spec.owner_label).filter(|t| !t.is_empty()) {
        return Some((t.clone(), OwnershipSource::Label));
    }
    if let Some(t) = w
        .annotations
        .get(&spec.deploy_annotation)
        .filter(|t| !t.is_empty())
    {
        return Some((t.clone(), OwnershipSource::DeployMetadata));
    }
    let path = w.annotations.get(&spec.source_path_annotation)?;
    let owners = codeowners_for(spec.codeowners.as_deref()?, path);
    // Prefer the first handle that maps to a known team; otherwise report the
    // raw handle so it surfaces as stale.
    owners
        .iter()
        .find_map(|h| {
            spec.teams
                .iter()
                .find(|t| t.handles.iter().any(|x| x == h))
                .map(|t| t.team.clone())
        })
        .or_else(|| owners.first().map(|h| h.to_string()))
        .map(|t| (t, OwnershipSource::Codeowners))
}

/// Derive the registry status from live workloads.
pub fn derive_status(
    spec: &ServiceOwnershipRegistrySpec,
    workloads: &[WorkloadMeta],
    previous: &ServiceOwnershipRegistryStatus,
    now: DateTime<Utc>,
) -> ServiceOwnershipRegistryStatus {
    let teams: HashMap<&str, &TeamRotation> =
        spec.teams.iter().map(|t| (t.team.as_str(), t)).collect();
    let prev: HashMap<&WorkloadRef, &str> = previous
        .entries
        .iter()
        .map(|e| (&e.workload, e.team.as_str()))
        .chain(
            previous
                .stale
                .iter()
                .map(|s| (&s.workload, s.team.as_str())),
        )
        .collect();

    let mut status = ServiceOwnershipRegistryStatus {
        history: previous.history.clone(),
        last_reconciled: Some(now),
        ..Default::default()
    };

    let mut sorted: Vec<&WorkloadMeta> = workloads
        .iter()
        .filter(|w| !spec.excluded_namespaces.contains(&w.workload.namespace))
        .collect();
    sorted.sort_by(|a, b| a.workload.cmp(&b.workload));

    for w in sorted {
        let resolved = resolve(spec, w);
        let team = resolved.as_ref().map(|(t, _)| t.clone());
        let previous_team = prev.get(&w.workload).map(|t| t.to_string());
        if team != previous_team {
            status.history.push(OwnershipChange {
                workload: w.workload.clone(),
                previous_team,
                team: team.clone(),
                source: resolved.as_ref().map(|(_, s)| *s),
                changed_at: now,
            });
        }

        match resolved {
            None => status.unowned.push(w.workload.clone()),
            Some((team, source)) => match teams.get(team.as_str()) {
                None => status.stale.push(StaleOwnership {
                    workload: w.workload.clone(),
                    team,
                    reason: "owner is not a registered team".into(),
                }),
                Some(t) if !t.live => status.stale.push(StaleOwnership {
                    workload: w.workload.clone(),
                    team,
                    reason: format!("rotation '{}' has no one on call", t.rotation),
                }),
                Some(t) => status.entries.push(OwnershipEntry {
                    workload: w.workload.clone(),
                    team,
                    rotation: t.rotation.clone(),
                    receiver: t.receiver.clone(),
                    source,
                }),
            },
        }
    }

    let limit = spec.history_limit;
    if status.history.len() > limit {
        status.history.drain(..status.history.len() - limit);
    }
    status.owned_count = status.entries.len() as u32;
    status.unowned_count = (status.unowned.len() + status.stale.len()) as u32;
    let complete = status.unowned_count == 0;
    status.conditions = vec![Condition::ready(
        complete,
        if complete {
            "AllOwned"
        } else {
            "OwnershipGaps"
        },
        &format!(
            "{} owned, {} unowned, {} stale",
            status.entries.len(),
            status.unowned.len(),
            status.stale.len()
        ),
    )];
    status
}

/// Ownership history of one workload, oldest first.
pub fn history_for<'a>(
    status: &'a ServiceOwnershipRegistryStatus,
    workload: &WorkloadRef,
) -> Vec<&'a OwnershipChange> {
    status
        .history
        .iter()
        .filter(|c| &c.workload == workload)
        .collect()
}

/// Workload label keys alerts carry, per Kubernetes kind.
const ALERT_WORKLOAD_LABELS: [(&str, &str); 3] = [
    ("deployment", "Deployment"),
    ("statefulset", "StatefulSet"),
    ("daemonset", "DaemonSet"),
];

/// Resolve an alert's owner from the registry using its `namespace` and
/// `deployment` / `statefulset` / `daemonset` labels.
pub fn attribute_alert<'a>(
    status: &'a ServiceOwnershipRegistryStatus,
    labels: &BTreeMap<String, String>,
) -> Option<&'a OwnershipEntry> {
    let ns = labels.get("namespace")?;
    ALERT_WORKLOAD_LABELS.iter().find_map(|(label, kind)| {
        let name = labels.get(*label)?;
        status.entries.iter().find(|e| {
            e.workload.namespace == *ns && e.workload.name == *name && e.workload.kind == *kind
        })
    })
}

/// Alertmanager child routes, one per team, matching the `team` label the
/// registry attaches to alerts. Unattributed alerts fall through to the
/// parent route's default receiver.
pub fn alertmanager_routes(spec: &ServiceOwnershipRegistrySpec) -> serde_json::Value {
    serde_json::Value::Array(
        spec.teams
            .iter()
            .map(|t| {
                serde_json::json!({
                    "receiver": t.receiver,
                    "matchers": [format!("team=\"{}\"", t.team)],
                    "continue": false
                })
            })
            .collect(),
    )
}

/// Alertmanager v2 alerts for unowned and stale workloads.
pub fn ownership_alerts(status: &ServiceOwnershipRegistryStatus) -> Vec<serde_json::Value> {
    let alert = |name: &str, w: &WorkloadRef, summary: String| {
        serde_json::json!({
            "labels": {
                "alertname": name,
                "severity": "warning",
                "namespace": w.namespace,
                "kind": w.kind,
                "workload": w.name,
            },
            "annotations": { "summary": summary }
        })
    };
    status
        .unowned
        .iter()
        .map(|w| {
            alert(
                "WorkloadUnowned",
                w,
                format!(
                    "{}/{} {} has no resolvable owner",
                    w.namespace, w.name, w.kind
                ),
            )
        })
        .chain(status.stale.iter().map(|s| {
            alert(
                "WorkloadOwnershipStale",
                &s.workload,
                format!("owner '{}': {}", s.team, s.reason),
            )
        }))
        .collect()
}

fn meta_of<K: Resource>(kind: &str, obj: &K) -> WorkloadMeta {
    let m = obj.meta();
    WorkloadMeta {
        workload: WorkloadRef {
            kind: kind.into(),
            namespace: m.namespace.clone().unwrap_or_default(),
            name: m.name.clone().unwrap_or_default(),
        },
        labels: m.labels.clone().unwrap_or_default(),
        annotations: m.annotations.clone().unwrap_or_default(),
    }
}

/// List all Deployments, StatefulSets and DaemonSets in the cluster.
pub async fn list_workloads(client: &Client) -> Result<Vec<WorkloadMeta>> {
    let lp = ListParams::default();
    let mut out = Vec::new();
    for d in Api::<Deployment>::all(client.clone()).list(&lp).await? {
        out.push(meta_of("Deployment", &d));
    }
    for s in Api::<StatefulSet>::all(client.clone()).list(&lp).await? {
        out.push(meta_of("StatefulSet", &s));
    }
    for d in Api::<DaemonSet>::all(client.clone()).list(&lp).await? {
        out.push(meta_of("DaemonSet", &d));
    }
    Ok(out)
}

/// Reconcile one registry: derive status from live workloads, patch it, and
/// alert on unowned/stale workloads in the same cycle.
pub async fn reconcile_ownership_registry(
    client: &Client,
    registry: &ServiceOwnershipRegistry,
) -> Result<ServiceOwnershipRegistryStatus> {
    let workloads = list_workloads(client).await?;
    let previous = registry.status.clone().unwrap_or_default();
    let status = derive_status(&registry.spec, &workloads, &previous, Utc::now());

    let api: Api<ServiceOwnershipRegistry> = Api::all(client.clone());
    api.patch_status(
        &registry.name_any(),
        &PatchParams::default(),
        &Patch::Merge(serde_json::json!({ "status": status })),
    )
    .await?;

    let alerts = ownership_alerts(&status);
    if let (Some(url), false) = (&registry.spec.alertmanager_url, alerts.is_empty()) {
        reqwest::Client::new()
            .post(format!("{}/api/v2/alerts", url.trim_end_matches('/')))
            .json(&alerts)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .inspect_err(|e| warn!(error = %e, "failed to send ownership alerts"))?;
    }

    info!(
        registry = %registry.name_any(),
        owned = status.owned_count,
        unowned = status.unowned_count,
        "ServiceOwnershipRegistry reconciled"
    );
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEOWNERS: &str = "\
*                   @maintainers
/charts/            @devops-team
/src/controller/    @kubernetes-experts
*.sql               @db-team # trailing comment
";

    fn spec() -> ServiceOwnershipRegistrySpec {
        let team = |name: &str, handle: &str, live: bool| TeamRotation {
            team: name.into(),
            handles: vec![handle.into()],
            rotation: format!("{name}-primary"),
            receiver: format!("{name}-pager"),
            live,
        };
        ServiceOwnershipRegistrySpec {
            owner_label: "stellar.org/owner".into(),
            deploy_annotation: "stellar.org/deployed-by-team".into(),
            source_path_annotation: "stellar.org/source-path".into(),
            codeowners: Some(CODEOWNERS.into()),
            teams: vec![
                team("platform", "@maintainers", true),
                team("devops", "@devops-team", true),
                team("k8s", "@kubernetes-experts", true),
                team("db", "@db-team", false),
            ],
            excluded_namespaces: vec!["kube-system".into()],
            alertmanager_url: None,
            history_limit: 100,
        }
    }

    fn wl(name: &str, labels: &[(&str, &str)], ann: &[(&str, &str)]) -> WorkloadMeta {
        let map = |kv: &[(&str, &str)]| {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        WorkloadMeta {
            workload: WorkloadRef {
                kind: "Deployment".into(),
                namespace: "stellar".into(),
                name: name.into(),
            },
            labels: map(labels),
            annotations: map(ann),
        }
    }

    #[test]
    fn codeowners_semantics() {
        assert!(codeowners_match("*", "src/main.rs"));
        assert!(codeowners_match(
            "/charts/",
            "charts/stellar-operator/values.yaml"
        ));
        assert!(!codeowners_match("/charts/", "docs/charts/x.md"));
        assert!(codeowners_match("*.sql", "db/migrations/001.sql"));
        assert!(codeowners_match(
            "/src/controller/mtls.rs",
            "src/controller/mtls.rs"
        ));
        assert_eq!(
            codeowners_for(CODEOWNERS, "src/controller/foo.rs"),
            vec!["@kubernetes-experts"]
        );
        assert_eq!(
            codeowners_for(CODEOWNERS, "README.md"),
            vec!["@maintainers"]
        );
    }

    #[test]
    fn precedence_label_then_deploy_then_codeowners() {
        let s = spec();
        let ws = vec![
            wl(
                "a",
                &[("stellar.org/owner", "devops")],
                &[("stellar.org/deployed-by-team", "k8s")],
            ),
            wl("b", &[], &[("stellar.org/deployed-by-team", "k8s")]),
            wl("c", &[], &[("stellar.org/source-path", "charts/foo")]),
        ];
        let st = derive_status(&s, &ws, &Default::default(), Utc::now());
        let got: Vec<_> = st
            .entries
            .iter()
            .map(|e| (e.workload.name.as_str(), e.team.as_str(), e.source))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a", "devops", OwnershipSource::Label),
                ("b", "k8s", OwnershipSource::DeployMetadata),
                ("c", "devops", OwnershipSource::Codeowners),
            ]
        );
        assert_eq!(st.conditions[0].status, "True");
    }

    #[test]
    fn unowned_and_stale_are_alerted_in_one_cycle() {
        let s = spec();
        let ws = vec![
            wl("orphan", &[], &[]),
            wl("ghost", &[("stellar.org/owner", "nobody")], &[]),
            wl("db", &[], &[("stellar.org/source-path", "db/schema.sql")]),
            WorkloadMeta {
                workload: WorkloadRef {
                    kind: "DaemonSet".into(),
                    namespace: "kube-system".into(),
                    name: "kube-proxy".into(),
                },
                ..Default::default()
            },
        ];
        let st = derive_status(&s, &ws, &Default::default(), Utc::now());
        assert_eq!(st.unowned.len(), 1);
        assert_eq!(st.stale.len(), 2);
        assert_eq!(st.unowned_count, 3);
        assert_eq!(st.conditions[0].status, "False");
        let alerts = ownership_alerts(&st);
        assert_eq!(alerts.len(), 3);
        assert_eq!(alerts[0]["labels"]["alertname"], "WorkloadUnowned");
        assert_eq!(alerts[0]["labels"]["workload"], "orphan");
    }

    #[test]
    fn history_tracks_changes_and_is_bounded() {
        let mut s = spec();
        let t0 = Utc::now();
        let st1 = derive_status(
            &s,
            &[wl("a", &[("stellar.org/owner", "devops")], &[])],
            &Default::default(),
            t0,
        );
        assert_eq!(st1.history.len(), 1);
        let st2 = derive_status(
            &s,
            &[wl("a", &[("stellar.org/owner", "devops")], &[])],
            &st1,
            t0,
        );
        assert_eq!(st2.history.len(), 1, "no change, no record");
        let st3 = derive_status(
            &s,
            &[wl("a", &[("stellar.org/owner", "k8s")], &[])],
            &st2,
            t0,
        );
        let h = history_for(&st3, &st3.entries[0].workload);
        assert_eq!(h.len(), 2);
        assert_eq!(h[1].previous_team.as_deref(), Some("devops"));
        assert_eq!(h[1].team.as_deref(), Some("k8s"));

        s.history_limit = 1;
        let st4 = derive_status(
            &s,
            &[wl("a", &[("stellar.org/owner", "platform")], &[])],
            &st3,
            t0,
        );
        assert_eq!(st4.history.len(), 1);
        assert_eq!(st4.history[0].team.as_deref(), Some("platform"));
    }

    #[test]
    fn alert_attribution_matches_registry() {
        let s = spec();
        let ws: Vec<_> = (0..100)
            .map(|i| {
                let team = ["platform", "devops", "k8s"][i % 3];
                wl(&format!("w{i}"), &[("stellar.org/owner", team)], &[])
            })
            .collect();
        let st = derive_status(&s, &ws, &Default::default(), Utc::now());
        assert_eq!(st.entries.len(), 100);
        for e in &st.entries {
            let labels = BTreeMap::from([
                ("namespace".to_string(), e.workload.namespace.clone()),
                ("deployment".to_string(), e.workload.name.clone()),
            ]);
            let got = attribute_alert(&st, &labels).unwrap();
            assert_eq!(got, e);
            assert!(s.teams.iter().any(|t| t.team == got.team && t.live));
        }
        let routes = alertmanager_routes(&s);
        assert_eq!(routes[1]["receiver"], "devops-pager");
        assert_eq!(routes[1]["matchers"][0], "team=\"devops\"");
    }
}
