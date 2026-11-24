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

//! kube-scheduler **extender** protocol (`filter`, `preempt`) backed by the fair-share engine.
//!
//! Configure kube-scheduler with `nodeCacheCapable: true` (see
//! `config/fair-share/scheduler-config.yaml`). Wire format follows
//! `k8s.io/kube-scheduler/extender/v1`.

use super::engine::{Engine, PodRec, Verdict};
use super::ResVec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

fn quantity(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(Value::as_str)
        .map(|s| {
            if key == "cpu" {
                super::parse_cpu(s)
            } else {
                super::parse_mem(s)
            }
            .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Effective pod request: sum of containers, raised to the largest init container per resource.
pub fn pod_request(pod: &Value) -> ResVec {
    let sum = |key: &str| -> ResVec {
        let mut tot = ResVec::ZERO;
        if let Some(cs) = pod
            .pointer(&format!("/spec/{key}"))
            .and_then(Value::as_array)
        {
            for c in cs {
                if let Some(r) = c.pointer("/resources/requests") {
                    tot = tot.add(ResVec::new(quantity(r, "cpu"), quantity(r, "memory")));
                }
            }
        }
        tot
    };
    let mut init_max = ResVec::ZERO;
    if let Some(cs) = pod
        .pointer("/spec/initContainers")
        .and_then(Value::as_array)
    {
        for c in cs {
            if let Some(r) = c.pointer("/resources/requests") {
                init_max = init_max.max(ResVec::new(quantity(r, "cpu"), quantity(r, "memory")));
            }
        }
    }
    sum("containers").max(init_max)
}

/// Convert a Kubernetes Pod JSON document into a [`PodRec`].
pub fn pod_from_json(pod: &Value, started: u64) -> Option<PodRec> {
    Some(PodRec {
        uid: pod.pointer("/metadata/uid")?.as_str()?.to_string(),
        namespace: pod.pointer("/metadata/namespace")?.as_str()?.to_string(),
        node: pod
            .pointer("/spec/nodeName")
            .and_then(Value::as_str)
            .map(str::to_string),
        req: pod_request(pod),
        priority: pod
            .pointer("/spec/priority")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        started,
    })
}

#[derive(Debug, Deserialize)]
pub struct ExtenderArgs {
    pub pod: Value,
    #[serde(default)]
    pub nodenames: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Default)]
pub struct ExtenderFilterResult {
    pub nodenames: Option<Vec<String>>,
    #[serde(rename = "failedNodes")]
    pub failed_nodes: HashMap<String, String>,
    #[serde(rename = "failedAndUnresolvableNodes")]
    pub failed_and_unresolvable: HashMap<String, String>,
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct MetaPod {
    pub uid: String,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MetaVictims {
    pub pods: Vec<Value>,
    #[serde(rename = "numPDBViolations", default)]
    pub num_pdb_violations: i64,
}
#[derive(Debug, Deserialize)]
pub struct ExtenderPreemptionArgs {
    pub pod: Value,
    #[serde(rename = "nodeNameToMetaVictims", default)]
    pub node_name_to_meta_victims: HashMap<String, MetaVictims>,
}
#[derive(Debug, Serialize, Default)]
pub struct ExtenderPreemptionResult {
    #[serde(rename = "nodeNameToMetaVictims")]
    pub node_name_to_meta_victims: HashMap<String, MetaVictims>,
}

/// Shared state of the extender service.
pub struct ExtenderState {
    pub engine: RwLock<Engine>,
    seen: RwLock<HashSet<String>>,
}

impl ExtenderState {
    pub fn new(engine: Engine) -> Arc<Self> {
        Arc::new(Self {
            engine: RwLock::new(engine),
            seen: RwLock::new(HashSet::new()),
        })
    }
}

/// `/filter`: admit or reject the pod for all candidate nodes (quota decisions are node-independent).
pub fn filter(state: &ExtenderState, args: &ExtenderArgs) -> ExtenderFilterResult {
    let nodes = args.nodenames.clone().unwrap_or_default();
    let Some(pod) = pod_from_json(&args.pod, 0) else {
        return ExtenderFilterResult {
            error: "pod is missing metadata.uid/namespace".into(),
            ..Default::default()
        };
    };
    let engine = state.engine.read().unwrap();
    let verdict = engine.admit(&pod);
    if verdict.allowed() {
        if let Some(created) = args
            .pod
            .pointer("/metadata/creationTimestamp")
            .and_then(Value::as_str)
        {
            let mut seen = state.seen.write().unwrap();
            if seen.len() > 20_000 {
                seen.clear();
            }
            if seen.insert(pod.uid.clone()) {
                if let Ok(t) = chrono::DateTime::parse_from_rfc3339(created) {
                    let d = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_milliseconds()
                        as f64
                        / 1000.0;
                    engine.metrics.observe_scheduling_delay(d.max(0.0));
                }
            }
        }
        return ExtenderFilterResult {
            nodenames: Some(nodes),
            ..Default::default()
        };
    }
    let reason = verdict.reason();
    let map: HashMap<String, String> = nodes.into_iter().map(|n| (n, reason.clone())).collect();
    match verdict {
        // Hitting a hard ceiling with *elastic* usage cannot be fixed by preempting other tenants
        // (NeedsReclaim, by contrast, is resolvable: it falls through to the failedNodes arm).
        Verdict::DenyMax { .. } => ExtenderFilterResult {
            nodenames: Some(vec![]),
            failed_and_unresolvable: map,
            ..Default::default()
        },
        _ => ExtenderFilterResult {
            nodenames: Some(vec![]),
            failed_nodes: map,
            ..Default::default()
        },
    }
}

/// `/preempt`: veto candidate victims that would breach guarantees or hierarchy boundaries.
pub fn preempt(state: &ExtenderState, args: &ExtenderPreemptionArgs) -> ExtenderPreemptionResult {
    let mut out = ExtenderPreemptionResult::default();
    let Some(pending) = pod_from_json(&args.pod, 0) else {
        return out;
    };
    let engine = state.engine.read().unwrap();
    for (node, victims) in &args.node_name_to_meta_victims {
        let recs: Option<Vec<PodRec>> = victims
            .pods
            .iter()
            .map(|p| {
                p.get("uid")
                    .and_then(Value::as_str)
                    .and_then(|u| engine.pod(u).cloned())
            })
            .collect();
        // Unknown victims (not yet observed) cannot be proven safe: veto the node.
        let Some(recs) = recs else { continue };
        let refs: Vec<&PodRec> = recs.iter().collect();
        if engine.check_preemption(&pending, &refs).is_ok() {
            out.node_name_to_meta_victims
                .insert(node.clone(), victims.clone());
        }
    }
    out
}

#[cfg(feature = "axum")]
pub mod http {
    use super::*;
    use axum::extract::State;
    use axum::http::{header, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};

    pub fn router(state: Arc<ExtenderState>) -> Router {
        Router::new()
            .route("/filter", post(|State(s): State<Arc<ExtenderState>>, Json(a): Json<ExtenderArgs>| async move { Json(filter(&s, &a)) }))
            .route(
                "/preempt",
                post(|State(s): State<Arc<ExtenderState>>, Json(a): Json<ExtenderPreemptionArgs>| async move { Json(preempt(&s, &a)) }),
            )
            .route("/healthz", get(|| async { "ok" }))
            .route(
                "/metrics",
                get(|State(s): State<Arc<ExtenderState>>| async move {
                    let body = s.engine.read().unwrap().render_metrics();
                    let r: Response = ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response();
                    let _ = StatusCode::OK;
                    r
                }),
            )
            .with_state(state)
    }
}
