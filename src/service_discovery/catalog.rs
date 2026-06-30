//! Service catalog with API documentation generation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::registry::ServiceRegistration;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub service_type: String,
    pub namespace: String,
    pub version: String,
    pub endpoint: String,
    pub port: u16,
    pub health_score: f64,
    pub api_docs_url: Option<String>,
    pub labels: HashMap<String, String>,
}

impl ServiceEntry {
    pub fn from_registration(reg: &ServiceRegistration) -> Self {
        Self {
            id: reg.id.clone(),
            name: reg.name.clone(),
            description: format!("{:?} service in namespace {}", reg.service_type, reg.namespace),
            service_type: format!("{:?}", reg.service_type),
            namespace: reg.namespace.clone(),
            version: reg.version.clone(),
            endpoint: reg.endpoint.clone(),
            port: reg.port,
            health_score: reg.health_score.score,
            api_docs_url: reg.annotations.get("docs/url").cloned(),
            labels: reg.labels.clone(),
        }
    }
}

/// Service catalog providing a browsable registry of all discovered services
pub struct ServiceCatalog {
    entries: HashMap<String, ServiceEntry>,
}

impl ServiceCatalog {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    pub fn rebuild(&mut self, services: &[ServiceRegistration]) {
        self.entries.clear();
        for svc in services {
            self.entries.insert(svc.id.clone(), ServiceEntry::from_registration(svc));
        }
    }

    pub fn get(&self, id: &str) -> Option<&ServiceEntry> {
        self.entries.get(id)
    }

    pub fn list(&self) -> Vec<&ServiceEntry> {
        self.entries.values().collect()
    }

    /// Export catalog as Markdown documentation
    pub fn to_markdown(&self) -> String {
        let mut md = String::from("# Stellar Service Catalog\n\n");
        md.push_str("| Name | Type | Namespace | Version | Health | Endpoint |\n");
        md.push_str("|------|------|-----------|---------|--------|----------|\n");
        let mut entries: Vec<&ServiceEntry> = self.entries.values().collect();
        entries.sort_by_key(|e| &e.name);
        for e in entries {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {:.0}% | {}:{} |\n",
                e.name, e.service_type, e.namespace, e.version,
                e.health_score * 100.0, e.endpoint, e.port,
            ));
        }
        md
    }
}

impl Default for ServiceCatalog {
    fn default() -> Self {
        Self::new()
    }
}
