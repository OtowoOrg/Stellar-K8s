use kube::CustomResourceExt;
use stellar_k8s::crd::StellarNode;

fn main() {
    print!("{}", serde_yaml::to_string(&StellarNode::crd()).unwrap());
}
