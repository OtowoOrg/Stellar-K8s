use kube::CustomResourceExt;
use stellar_k8s::crd::{ContractInstance, ContractWASM};

fn main() {
    print!("{}", serde_yaml::to_string(&ContractWASM::crd()).unwrap());
    print!(
        "---\n{}",
        serde_yaml::to_string(&ContractInstance::crd()).unwrap()
    );
}
