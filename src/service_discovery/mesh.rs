//! Service mesh integration hooks (Istio/Linkerd compatible annotations)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Annotations injected onto Kubernetes Service/Pod resources for service mesh integration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MeshAnnotations {
    pub annotations: HashMap<String, String>,
}

impl MeshAnnotations {
    pub fn istio(service_name: &str, version: &str) -> Self {
        let mut a = HashMap::new();
        a.insert("sidecar.istio.io/inject".into(), "true".into());
        a.insert("istio.io/rev".into(), "default".into());
        a.insert("version".into(), version.into());
        Self { annotations: a }
    }

    pub fn linkerd(service_name: &str) -> Self {
        let mut a = HashMap::new();
        a.insert("linkerd.io/inject".into(), "enabled".into());
        a.insert("config.linkerd.io/proxy-log-level".into(), "warn".into());
        Self { annotations: a }
    }
}

/// Facade for service mesh integration — adds mesh-specific annotations to resources
pub struct ServiceMeshIntegration {
    mesh_type: MeshType,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MeshType {
    Istio,
    Linkerd,
    None,
}

impl ServiceMeshIntegration {
    pub fn new(mesh_type: MeshType) -> Self {
        Self { mesh_type }
    }

    pub fn annotations_for(&self, service_name: &str, version: &str) -> MeshAnnotations {
        match self.mesh_type {
            MeshType::Istio => MeshAnnotations::istio(service_name, version),
            MeshType::Linkerd => MeshAnnotations::linkerd(service_name),
            MeshType::None => MeshAnnotations::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_istio_annotations() {
        let mesh = ServiceMeshIntegration::new(MeshType::Istio);
        let ann = mesh.annotations_for("horizon", "v1.0");
        assert_eq!(ann.annotations.get("sidecar.istio.io/inject"), Some(&"true".to_string()));
    }

    #[test]
    fn test_none_mesh_empty_annotations() {
        let mesh = ServiceMeshIntegration::new(MeshType::None);
        let ann = mesh.annotations_for("core", "v1.0");
        assert!(ann.annotations.is_empty());
    }
}
