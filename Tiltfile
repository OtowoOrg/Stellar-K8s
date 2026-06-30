# Tilt local development for Stellar-K8s
# Usage: tilt up

allow_k8s_contexts('kind-stellar', 'k3d-stellar')

local_resource(
    'build-operator',
    'cargo build --bin stellar-operator --features rest-api,metrics,admission-webhook,k8s-v1-30',
    deps=['src', 'Cargo.toml'],
    ignore=['target'],
)

docker_build(
    'stellar-operator',
    '.',
    dockerfile='Dockerfile',
    only=['target/release/stellar-operator', 'target/release/kubectl-stellar'],
    live_update=[
        sync('./target/debug/stellar-operator', '/usr/local/bin/stellar-operator'),
    ],
)

k8s_yaml('config/manifests/operator-deployment.yaml')
k8s_resource('stellar-operator', port_forwards=['8080:8080', '9090:9090'])

local_resource(
    'cargo-watch',
    serve_cmd='cargo watch -x "run --bin stellar-operator --features rest-api,metrics,admission-webhook,k8s-v1-30"',
    resource_deps=['build-operator'],
    allow_parallel=True,
)
