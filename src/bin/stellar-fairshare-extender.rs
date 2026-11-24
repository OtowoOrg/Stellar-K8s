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

//! `stellar-fairshare-extender` — kube-scheduler extender enforcing hierarchical
//! fair-share quotas from `TenantQuota` resources (#1959).
//!
//! It keeps a pod/node/quota view by polling the API server and serves
//! `/filter`, `/preempt`, `/metrics`. An invalid `TenantQuota` update never replaces
//! the active configuration (last known good is kept and `config_valid` drops to 0).

use clap::Parser;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::api::{Api, ListParams};
use kube::{Client, CustomResourceExt};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use stellar_k8s::fair_share::crd::TenantQuota;
use stellar_k8s::fair_share::engine::Engine;
use stellar_k8s::fair_share::extender::{http, pod_from_json, ExtenderState};
use stellar_k8s::fair_share::tree::Tree;
use stellar_k8s::fair_share::{parse_cpu, parse_mem, ResVec};

#[derive(Parser)]
#[command(
    name = "stellar-fairshare-extender",
    version,
    about = "Hierarchical fair-share kube-scheduler extender"
)]
struct Args {
    #[arg(long, env = "FAIRSHARE_LISTEN", default_value = "0.0.0.0:8888")]
    listen: SocketAddr,
    /// Seconds between API refreshes of quotas, pods and node capacity.
    #[arg(long, default_value_t = 2)]
    refresh_secs: u64,
    /// Print the TenantQuota CRD YAML and exit.
    #[arg(long)]
    print_crd: bool,
}

fn node_allocatable(n: &Node) -> ResVec {
    let ready = n
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|c| c.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
        .unwrap_or(false);
    let unschedulable = n
        .spec
        .as_ref()
        .and_then(|s| s.unschedulable)
        .unwrap_or(false);
    if !ready || unschedulable {
        return ResVec::ZERO;
    }
    let a = n.status.as_ref().and_then(|s| s.allocatable.as_ref());
    let get = |k: &str| a.and_then(|m| m.get(k)).map(|q| q.0.clone());
    ResVec::new(
        get("cpu").and_then(|s| parse_cpu(&s).ok()).unwrap_or(0),
        get("memory").and_then(|s| parse_mem(&s).ok()).unwrap_or(0),
    )
}

async fn refresh(
    client: &Client,
    state: &ExtenderState,
    order: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let quotas: Vec<TenantQuota> = Api::<TenantQuota>::all(client.clone())
        .list(&ListParams::default())
        .await?
        .items;
    let specs: Vec<_> = quotas
        .iter()
        .filter_map(|q| q.metadata.name.clone().map(|n| (n, q.spec.clone())))
        .collect();
    let tree = Tree::build(&specs);
    let nodes = Api::<Node>::all(client.clone())
        .list(&ListParams::default())
        .await?
        .items;
    let capacity = nodes
        .iter()
        .fold(ResVec::ZERO, |a, n| a.add(node_allocatable(n)));
    let pods = Api::<Pod>::all(client.clone())
        .list(&ListParams::default())
        .await?
        .items;
    let (mut running, mut pending) = (Vec::new(), Vec::new());
    for p in pods {
        let phase = p
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_default();
        if phase == "Succeeded"
            || phase == "Failed"
            || p.metadata.deletion_timestamp.is_some() && phase != "Running"
        {
            continue;
        }
        let Ok(v) = serde_json::to_value(&p) else {
            continue;
        };
        *order += 1;
        if let Some(rec) = pod_from_json(&v, *order) {
            if rec.node.is_some() {
                running.push(rec)
            } else {
                pending.push(rec)
            }
        }
    }
    let mut engine = state.engine.write().unwrap();
    match tree {
        Ok(t) => {
            for e in t.check_capacity(capacity) {
                tracing::warn!(%e, "quota configuration is oversubscribed relative to cluster capacity");
            }
            engine.set_tree(t);
        }
        Err(errs) => {
            engine.note_invalid_config();
            for e in errs {
                tracing::error!(error = %e, "TenantQuota configuration rejected; keeping last known good");
            }
        }
    }
    engine.set_capacity(capacity);
    engine.replace_pods(running, pending);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.print_crd {
        print!("{}", serde_yaml::to_string(&TenantQuota::crd())?);
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
    let client = Client::try_default().await?;
    let empty = Tree::build(&[]).expect("empty tree is valid");
    let state = ExtenderState::new(Engine::new(empty, ResVec::ZERO));
    let st = Arc::clone(&state);
    let every = Duration::from_secs(args.refresh_secs.max(1));
    tokio::spawn(async move {
        let mut order = 0u64;
        loop {
            if let Err(e) = refresh(&client, &st, &mut order).await {
                tracing::warn!(error = %e, "refresh failed; serving last known state");
            }
            tokio::time::sleep(every).await;
        }
    });
    let l = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, "fair-share extender listening");
    axum::serve(l, http::router(state)).await?;
    Ok(())
}
