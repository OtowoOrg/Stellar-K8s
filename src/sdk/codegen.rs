//! Code generation utilities for CRD-based controller scaffolding.

use serde::{Deserialize, Serialize};

/// Generated controller boilerplate metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControllerStub {
    pub crd_group: String,
    pub crd_version: String,
    pub crd_kind: String,
    pub module_name: String,
    pub reconciler_fn: String,
}

/// Generate a controller reconciler stub from CRD metadata.
pub fn generate_controller_stub(group: &str, version: &str, kind: &str) -> ControllerStub {
    let module_name = to_snake_case(kind);
    let reconciler_fn = format!("reconcile_{module_name}");
    ControllerStub {
        crd_group: group.to_string(),
        crd_version: version.to_string(),
        crd_kind: kind.to_string(),
        module_name,
        reconciler_fn,
    }
}

/// Render Rust source for a reconciler module stub.
pub fn render_controller_source(stub: &ControllerStub) -> String {
    format!(
        r#"//! Auto-generated reconciler stub for {kind}
use kube::{{Client, ResourceExt}};
use tracing::info;

use crate::crd::{kind}::{kind};

pub async fn {reconciler_fn}(client: &Client, resource: &{kind}) -> crate::error::Result<()> {{
    let name = resource.name_any();
    let namespace = resource.namespace().unwrap_or_else(|| "default".to_string());
    info!(%name, %namespace, "reconciling {kind}");
    let _ = client;
    Ok(())
}}
"#,
        kind = stub.crd_kind,
        reconciler_fn = stub.reconciler_fn,
    )
}

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_stellar_secret_stub() {
        let stub = generate_controller_stub("stellar.org", "v1alpha1", "StellarSecret");
        assert_eq!(stub.module_name, "stellar_secret");
        assert_eq!(stub.reconciler_fn, "reconcile_stellar_secret");
    }

    #[test]
    fn render_includes_kind_name() {
        let stub = generate_controller_stub("stellar.org", "v1alpha1", "StellarRegistry");
        let src = render_controller_source(&stub);
        assert!(src.contains("reconcile_stellar_registry"));
        assert!(src.contains("StellarRegistry"));
    }
}
