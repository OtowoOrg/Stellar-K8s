# Developer Onboarding Guide

This guide helps new contributors become productive with Stellar-K8s.

## 1. Welcome

Stellar-K8s is a Kubernetes operator for managing Stellar Core infrastructure using `StellarNode` custom resources.

## 2. Setup Your Environment

### Linux
- Install `kubectl`, `kind` or `minikube`, `docker`, `rust`, and `cargo`.
- Run `cargo test` and `make lint` after cloning the repository.

### macOS
- Use Homebrew to install dependencies: `brew install kubectl kind docker rustup`.
- Follow the same validation steps as Linux.

### Windows / WSL2
- Use WSL2 for development.
- Install `kubectl`, `docker`, and `rustup` inside WSL.
- Run `docs/installation-wsl2.md` for additional guidance.

## 3. Repository Tour

### Key directories
- `src/` — operator implementation and controller logic
- `charts/` — Helm charts for deploying Stellar-K8s
- `docs/` — project documentation and runbooks
- `tests/` — integration and end-to-end test suites
- `examples/` — sample manifests and deployment examples

### Important files
- `Cargo.toml` — Rust package metadata
- `README.md` — project overview and quick start
- `docs/api-reference.md` — CRD schema and field reference

## 4. Interactive Tutorials

### Tutorial 1: Build and Run Locally
1. Clone the repo.
2. Run `cargo build`.
3. Start a local Kubernetes cluster with `kind create cluster`.
4. Deploy the operator using `make deploy` or `helm install`.
5. Apply an example `StellarNode` manifest from `examples/`.

### Tutorial 2: Add a New Feature
1. Open the operator controller in `src/`
2. Locate the reconciliation loop and CRD watch logic.
3. Add a new field to the CRD spec and update the generated docs.
4. Run `make test` and `make generate-api-docs`.

### Tutorial 3: Debug a Reconciliation Failure
1. Trigger a failing `StellarNode` deployment.
2. Inspect the operator pod logs with `kubectl logs`.
3. Check the `StellarNode` status conditions.
4. Confirm the webhook validation behavior.

## 5. Coding Standards and Best Practices

- Follow Rust formatting with `cargo fmt`.
- Run linting and static analysis before submitting changes.
- Keep CRD and API docs in sync with schema changes.
- Add tests for both controller logic and API behavior.

## 6. Testing Strategy

- Unit tests: `cargo test` for Rust modules.
- Integration tests: `tests/` for CRD reconciliation and Kubernetes behavior.
- E2E tests: deploy the operator in a cluster and validate application workflows.

## 7. Debugging Guide

- Use `kubectl describe` and `kubectl get events` for failed workloads.
- Confirm CRD status conditions and operator readiness.
- Use `cargo test` for code-level issues and `kubectl logs` for runtime issues.
- Review `docs/faq.md` and troubleshooting guides before filing issues.
