//! Helpers for resolving the Kubernetes infrastructure backing a StellarNode workload.

use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{api::Api, Client, ResourceExt};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

const FEATURE_PREFIX: &str = "feature.node.kubernetes.io/";
const CPU_VENDOR_LABEL: &str = "feature.node.kubernetes.io/cpu-model.vendor_id";
const CPU_FAMILY_LABEL: &str = "feature.node.kubernetes.io/cpu-model.family";
const CPU_MODEL_LABEL: &str = "feature.node.kubernetes.io/cpu-model.id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodInfraAssignment {
    pub pod_name: String,
    pub kubernetes_node: Option<String>,
    pub hardware_generation: String,
    pub feature_labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfraSummary {
    pub assignments: Vec<PodInfraAssignment>,
}

impl InfraSummary {
    pub fn hardware_generation_label(&self) -> String {
        let generations: BTreeSet<String> = self
            .assignments
            .iter()
            .map(|assignment| assignment.hardware_generation.clone())
            .filter(|generation| generation != "unknown")
            .collect();

        match generations.len() {
            0 => "unknown".to_string(),
            1 => generations
                .into_iter()
                .next()
                .unwrap_or_else(|| "unknown".to_string()),
            _ => "mixed".to_string(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }
}

pub async fn resolve_stellar_node_infra(
    client: &Client,
    node: &StellarNode,
) -> Result<InfraSummary> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let label_selector = format!(
        "app.kubernetes.io/instance={},app.kubernetes.io/name=stellar-node",
        node.name_any()
    );

    let pods_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let nodes_api: Api<Node> = Api::all(client.clone());
    let pods = pods_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await
        .map_err(Error::KubeError)?;

    let mut assignments = Vec::with_capacity(pods.items.len());

    for pod in pods.items {
        let pod_name = pod.name_any();
        let kubernetes_node = pod.spec.as_ref().and_then(|spec| spec.node_name.clone());

        let (hardware_generation, feature_labels) = match kubernetes_node.as_deref() {
            Some(node_name) => match nodes_api.get(node_name).await {
                Ok(kube_node) => hardware_details_from_node(&kube_node),
                Err(kube::Error::Api(err)) if err.code == 404 => {
                    ("unknown".to_string(), BTreeMap::new())
                }
                Err(err) => return Err(Error::KubeError(err)),
            },
            None => ("pending".to_string(), BTreeMap::new()),
        };

        assignments.push(PodInfraAssignment {
            pod_name,
            kubernetes_node,
            hardware_generation,
            feature_labels,
        });
    }

    Ok(InfraSummary { assignments })
}

pub fn hardware_details_from_node(node: &Node) -> (String, BTreeMap<String, String>) {
    let labels = node.metadata.labels.as_ref().cloned().unwrap_or_default();
    let feature_labels = labels
        .into_iter()
        .filter(|(key, _)| key.starts_with(FEATURE_PREFIX))
        .collect::<BTreeMap<_, _>>();

    let generation = infer_hardware_generation(&feature_labels);
    (generation, feature_labels)
}

pub fn infer_hardware_generation(feature_labels: &BTreeMap<String, String>) -> String {
    // NFD's built-in CPU source exposes vendor/family/model labels consistently, but not a
    // universal "generation" label. We prefer an explicit generation label when present and
    // otherwise map well-known vendor/family/model tuples to operator-friendly names.
    for (key, value) in feature_labels {
        let key_lower = key.to_ascii_lowercase();
        let value_lower = value.to_ascii_lowercase();

        if key_lower.contains("generation")
            && (key_lower.contains("cpu")
                || key_lower.contains("instance")
                || key_lower.contains("microarchitecture"))
            && !value.trim().is_empty()
        {
            return sanitize_generation(value);
        }

        for candidate in [
            "graviton 4",
            "graviton4",
            "graviton 3",
            "graviton3",
            "graviton 2",
            "graviton2",
            "sapphire rapids",
            "sapphirerapids",
            "icelake",
            "ice lake",
            "cascadelake",
            "cascade lake",
            "skylake",
            "zen 4",
            "zen4",
            "zen 3",
            "zen3",
        ] {
            if key_lower.contains(candidate) || value_lower.contains(candidate) {
                return sanitize_generation(candidate);
            }
        }
    }

    let vendor = feature_labels
        .get(CPU_VENDOR_LABEL)
        .map(|value| value.to_ascii_lowercase());
    let family = feature_labels.get(CPU_FAMILY_LABEL).map(String::as_str);
    let model = feature_labels.get(CPU_MODEL_LABEL).map(String::as_str);

    match (vendor.as_deref(), family, model) {
        (Some("genuineintel"), Some("6"), Some("106" | "108")) => "Intel Icelake".to_string(),
        (Some("genuineintel"), Some("6"), Some("143")) => "Intel Sapphire Rapids".to_string(),
        (Some("genuineintel"), Some("6"), Some("85")) => "Intel Skylake/Cascadelake".to_string(),
        (Some("authenticamd"), Some("25"), Some("17")) => "AMD Zen 3".to_string(),
        (Some("authenticamd"), Some("25"), Some("97")) => "AMD Zen 4".to_string(),
        (Some("arm") | Some("0x41"), _, _) => "ARM (generation unknown)".to_string(),
        _ => match (vendor, family, model) {
            (Some(vendor), Some(family), Some(model)) => {
                format!(
                    "{} family {} model {}",
                    normalize_vendor(&vendor),
                    family,
                    model
                )
            }
            _ => "unknown".to_string(),
        },
    }
}

fn sanitize_generation(value: &str) -> String {
    let cleaned = value
        .replace("graviton4", "Graviton 4")
        .replace("graviton3", "Graviton 3")
        .replace("graviton2", "Graviton 2")
        .replace("sapphirerapids", "Sapphire Rapids")
        .replace("icelake", "Icelake")
        .replace("cascadelake", "Cascadelake")
        .replace("zen4", "Zen 4")
        .replace("zen3", "Zen 3");

    title_case(cleaned.trim())
}

fn normalize_vendor(vendor: &str) -> &str {
    match vendor {
        "genuineintel" => "Intel",
        "authenticamd" => "AMD",
        "arm" | "0x41" => "ARM",
        _ => vendor,
    }
}

fn title_case(input: &str) -> String {
    input
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_uppercase().to_string();
                    out.push_str(&chars.as_str().to_ascii_lowercase());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_known_intel_generation_from_nfd_cpu_model_labels() {
        let labels = BTreeMap::from([
            (CPU_VENDOR_LABEL.to_string(), "GenuineIntel".to_string()),
            (CPU_FAMILY_LABEL.to_string(), "6".to_string()),
            (CPU_MODEL_LABEL.to_string(), "106".to_string()),
        ]);

        assert_eq!(infer_hardware_generation(&labels), "Intel Icelake");
    }

    #[test]
    fn prefers_explicit_generation_like_graviton() {
        let labels = BTreeMap::from([(
            "feature.node.kubernetes.io/custom-cpu.generation".to_string(),
            "graviton3".to_string(),
        )]);

        assert_eq!(infer_hardware_generation(&labels), "Graviton 3");
    }

    #[test]
    fn falls_back_to_vendor_family_model_when_unknown() {
        let labels = BTreeMap::from([
            (CPU_VENDOR_LABEL.to_string(), "GenuineIntel".to_string()),
            (CPU_FAMILY_LABEL.to_string(), "6".to_string()),
            (CPU_MODEL_LABEL.to_string(), "42".to_string()),
        ]);

        assert_eq!(
            infer_hardware_generation(&labels),
            "Intel family 6 model 42"
        );
    }

    #[test]
    fn summary_collapses_mixed_generations() {
        let summary = InfraSummary {
            assignments: vec![
                PodInfraAssignment {
                    pod_name: "a".to_string(),
                    kubernetes_node: Some("node-a".to_string()),
                    hardware_generation: "Intel Icelake".to_string(),
                    feature_labels: BTreeMap::new(),
                },
                PodInfraAssignment {
                    pod_name: "b".to_string(),
                    kubernetes_node: Some("node-b".to_string()),
                    hardware_generation: "Graviton 3".to_string(),
                    feature_labels: BTreeMap::new(),
                },
            ],
        };

        assert_eq!(summary.hardware_generation_label(), "mixed");
    }
}
