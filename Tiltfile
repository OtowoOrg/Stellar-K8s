# Tilt local development for Stellar-K8s
# Usage: tilt up

allow_k8s_contexts('kind-stellar', 'k3d-stellar')

local_resource(
    'build-operator',
    'cargo build --release --bin stellar-operator --features rest-api,metrics,admission-webhook,k8s-v1-30',
    deps=['src', 'Cargo.toml'],
    ignore=['target'],
)

docker_build(
    'stellar-operator',
    '.',
    dockerfile='Dockerfile',
    target='runtime-local',
    only=['target/release/stellar-operator', 'target/release/kubectl-stellar'],
    live_update=[
        sync('./target/release/stellar-operator', '/stellar-operator'),
    ],
)

k8s_yaml(helm(
    'charts/stellar-operator',
    name='stellar-operator',
    values=['installCRDs=true'],
))
k8s_resource('stellar-operator', port_forwards=['8080:8080', '9090:9090'])
