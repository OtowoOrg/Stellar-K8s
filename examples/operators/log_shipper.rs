//! Example sidecar injector operator using the Stellar-K8s SDK.

use async_trait::async_trait;
use stellar_k8s::plugin_sdk::{InjectedSidecar, ReconcileContext, SidecarInjector};

pub struct LogShipperInjector;

#[async_trait]
impl SidecarInjector for LogShipperInjector {
    fn name(&self) -> &str {
        "log-shipper"
    }

    async fn sidecars(&self, _ctx: &ReconcileContext) -> Vec<InjectedSidecar> {
        vec![InjectedSidecar {
            name: "log-shipper".into(),
            image: "fluent/fluent-bit:3.0".into(),
            ..Default::default()
        }]
    }
}
