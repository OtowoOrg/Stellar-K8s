//! Example admission policy hook using registry admission checks.
//!
//! Call [`check_admission`] directly — there is no thin `validate_image` wrapper.

#[cfg(test)]
mod tests {
    use kube::core::ObjectMeta;
    use stellar_k8s::controller::check_admission;
    use stellar_k8s::crd::stellar_registry::{
        AdmissionPolicy, ScanningConfig, SigningConfig, StellarRegistry, StellarRegistrySpec,
        VulnerabilitySummary,
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
        assert!(check_admission(
            &sample_registry(),
            "app:v1",
            true,
            &VulnerabilitySummary::default()
        )
        .is_ok());
    }
}
