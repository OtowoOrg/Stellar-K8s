use kube::CustomResourceExt;
use std::fs;
use std::path::Path;
use stellar_k8s::crd::{
    StellarAIOps, StellarAutoscaler, StellarBackup, StellarBenchmark, StellarBenchmarkReport,
    StellarDRDrill, StellarFederation, StellarGitOpsConfig, StellarNode, StellarObservability,
    StellarRegistry, StellarRestore, StellarSecurityPolicy, StellarUpgrade,
};

fn main() {
    let crds = vec![
        ("stellarnode-crd.yaml", StellarNode::crd()),
        ("stellarautoscaler-crd.yaml", StellarAutoscaler::crd()),
        ("stellarbenchmark-crd.yaml", StellarBenchmark::crd()),
        (
            "stellarbenchmarkreport-crd.yaml",
            StellarBenchmarkReport::crd(),
        ),
        ("stellardr-crd.yaml", StellarBackup::crd()),
        ("stellarfederation-crd.yaml", StellarFederation::crd()),
        ("stellargitopsconfig-crd.yaml", StellarGitOpsConfig::crd()),
        ("stellarobservability-crd.yaml", StellarObservability::crd()),
        (
            "stellarsecuritypolicy-crd.yaml",
            StellarSecurityPolicy::crd(),
        ),
        ("stellarupgrade-crd.yaml", StellarUpgrade::crd()),
        ("stellaraiops-crd.yaml", StellarAIOps::crd()),
    ];

    let config_dir = Path::new("config/crd");
    fs::create_dir_all(config_dir).expect("Failed to create config/crd directory");

    for (filename, crd) in crds {
        let path = config_dir.join(filename);
        let yaml = serde_yaml::to_string(&crd).expect("Failed to serialize CRD");
        fs::write(&path, yaml).expect(&format!("Failed to write {}", filename));
        println!("Generated {}", path.display());
    }
}
