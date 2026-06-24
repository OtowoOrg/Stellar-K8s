//! Example admission policy hook using registry admission checks.

use stellar_k8s::controller::check_admission;
use stellar_k8s::crd::stellar_registry::{StellarRegistry, VulnerabilitySummary};

pub fn validate_image(registry: &StellarRegistry, image: &str, signed: bool) -> Result<(), String> {
    check_admission(registry, image, signed, &VulnerabilitySummary::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::core::ObjectMeta;
    use stellar_k8s::crd::stellar_registry::{
        AdmissionPolicy, RegistryMirror, ScanningConfig, SigningConfig, StellarRegistrySpec,
    };

    fn sample_registry() -> StellarRegistry {
        StellarRegistry {
            metadata: ObjectMeta {
                name: Some("reg".into()),
                namespace: Some("stellar".into()),
                ..Default::default()
            },
            spec: StellarRegistrySpec {
                endpoint: "registry.example.com".into(),
                scanning: ScanningConfig::default(),
                signing: SigningConfig {
                    require_signature: false,
                    ..Default::default()
                },
                admission: AdmissionPolicy::default(),
                mirrors: vec![],
                garbage_collection: None,
                proxy: None,
                auto_patch: None,
            },
            status: None,
        }
    }

    #[test]
    fn allows_signed_image() {
        assert!(validate_image(&sample_registry(), "app:v1", true).is_ok());
    }
}
