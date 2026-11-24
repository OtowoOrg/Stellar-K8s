# Local Development with kind

This guide takes you from a clean machine to a running Stellar-K8s operator on a
local [kind](https://kind.sigs.k8s.io/) cluster, with hot-reloading and
integration tests. Target time on a machine that already has Docker: **under 15
minutes**, most of it the first Rust build.

If you would rather use k3d, see [k3d local development](../development.md).
For the full reference on building, testing and contributing, see
[DEVELOPMENT.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/DEVELOPMENT.md).

---

## Contents

- [Pinned versions](#pinned-versions)
- [Platform setup](#platform-setup)
- [The 15-minute path](#the-15-minute-path)
- [Hot-reloading the operator](#hot-reloading-the-operator)
- [Integration tests against kind](#integration-tests-against-kind)
- [Makefile shortcuts](#makefile-shortcuts)
- [Diagnostics](#diagnostics)
- [Teardown](#teardown)

---

## Pinned versions

Use these versions. They are the ones this repository actually builds and tests
against; anything older is untested.

| Tool | Version | Where it comes from |
| --- | --- | --- |
| Rust | stable **1.92+** (minimum 1.88) | `README.md` and `CONTRIBUTING.md` state 1.88+; the CI MSRV job pins `1.92` (`.github/workflows/ci.yml`) |
| Docker | **24.0+** with BuildKit enabled, Compose v2 | `make quickstart` builds with `DOCKER_BUILDKIT=1`; `docker-compose.dev.yml` is a Compose overlay |
| kind | **0.20.0+** | `DEVELOPMENT.md` install snippet; CI provisions clusters with `helm/kind-action@v1.14.0` |
| kubectl | **1.30.x** | the operator compiles against `k8s-openapi` feature `v1_30` (`Cargo.toml`), so match the cluster's minor version |
| Helm | **3.14.0** | pinned in CI via `azure/setup-helm@v4` (`.github/workflows/ci.yml`) |

> **Note on Rust versions.** The repository currently names four different Rust
> versions: 1.88 in `README.md`/`CONTRIBUTING.md`, 1.92 in the CI MSRV job,
> 1.93 in the `cargo-chef` base image in `Dockerfile`, and 1.94 in
> `Dockerfile.dev`. Local development on stable 1.92 or newer satisfies all of
> them. Consolidating those into a single `rust-version` field in `Cargo.toml`
> would be a good follow-up.

Verify everything at once before you start:

```bash
rustc --version   # 1.92.0 or newer
docker --version  # 24.x or newer
docker ps         # must succeed: the daemon has to be running
kind version      # 0.20.0 or newer
kubectl version --client
helm version --short  # v3.14.x
```

---

## Platform setup

### Linux

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Docker Engine — follow https://docs.docker.com/engine/install/ for your distro,
# then allow non-root use so `docker ps` works without sudo:
sudo usermod -aG docker "$USER"   # log out and back in afterwards

# kind
curl -Lo ./kind https://kind.sigs.k8s.io/dl/v0.20.0/kind-linux-amd64
chmod +x ./kind && sudo mv ./kind /usr/local/bin/kind

# kubectl (pick the 1.30 line to match the operator's k8s-openapi features)
curl -LO "https://dl.k8s.io/release/v1.30.0/bin/linux/amd64/kubectl"
chmod +x kubectl && sudo mv kubectl /usr/local/bin/

# Helm
curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
```

Raise the inotify limits before running `kind` or `cargo watch`. Both consume
watches aggressively, and the default limits on most distributions are low
enough that clusters fail to start with cryptic errors:

```bash
sudo sysctl fs.inotify.max_user_watches=524288
sudo sysctl fs.inotify.max_user_instances=512
# Persist across reboots:
echo -e "fs.inotify.max_user_watches=524288\nfs.inotify.max_user_instances=512" \
  | sudo tee /etc/sysctl.d/99-kind.conf
```

### macOS

```bash
brew install rustup-init && rustup-init   # or the curl installer above
brew install kind kubectl helm
brew install --cask docker                # Docker Desktop
```

Start Docker Desktop and give it enough headroom before creating a cluster:
**Settings → Resources → at least 4 CPUs, 8 GB memory, 40 GB disk**. The
defaults (2 CPU / 2 GB on older installs) are not enough to build the operator
and run a kind cluster at the same time.

Apple Silicon: everything below works natively on arm64. The images the
operator builds are single-arch by default; use `make docker-multiarch` only
when you need to push a cross-platform image.

### Windows (WSL2)

Do all development **inside** the WSL2 filesystem. Working out of `/mnt/c`
makes Rust builds several times slower and breaks file-change events, so
hot-reloading will silently not fire.

1. Install WSL2 and a distro (PowerShell as Administrator):
   ```powershell
   wsl --install -d Ubuntu-22.04
   ```
2. Install Docker Desktop, then enable **Settings → Resources → WSL Integration**
   for your distro. `docker ps` must succeed from inside WSL.
3. Clone into the Linux filesystem, e.g. `~/src/stellar-k8s`, **not** `/mnt/c/...`.
4. Follow the Linux steps above for Rust, kind, kubectl and Helm.
5. Cap WSL2's memory so a kind cluster plus a Rust build does not exhaust the
   host. Create `C:\Users\<you>\.wslconfig`:
   ```ini
   [wsl2]
   memory=8GB
   processors=4
   swap=2GB
   ```
   Then `wsl --shutdown` from PowerShell and reopen your shell.

---

## The 15-minute path

From a clean clone:

```bash
git clone https://github.com/agnesnaomiolim-cloud/Stellar-K8s.git
cd Stellar-K8s

# 1. Toolchain: rustup components, cargo-audit, cargo-watch, pre-commit hooks
make dev-setup

# 2. Everything else: kind cluster, operator image, CRD, Helm release, sample node
make quickstart
```

`make quickstart` is the whole local environment in one target. It:

1. checks that `kind`, `kubectl` and `helm` are on `PATH` and fails early if not;
2. creates the kind cluster **`stellar-dev`** (reuses it if it already exists);
3. runs `make build` (`cargo build --release --locked`);
4. builds the `stellar-operator:dev` image from the `runtime-local` stage of
   `Dockerfile`, which copies the host-built binaries rather than rebuilding
   them in the container;
5. `kind load docker-image` to push that image into the cluster;
6. applies `config/crd/stellarnode-crd.yaml`;
7. creates the `stellar-system` namespace;
8. `helm upgrade --install` from `charts/stellar-operator` with
   `image.tag=dev` and `image.pullPolicy=Never`;
9. applies `config/samples/test-stellarnode.yaml`.

Verify:

```bash
kubectl get stellarnode -n stellar-system -w
kubectl get deploy,sts,svc,pvc -n stellar-system
kubectl logs -n stellar-system deploy/stellar-operator -f
```

**Where the time goes.** Steps 1–2 and 5–9 take roughly two minutes. Step 3 is
the cold Rust release build, which dominates: expect 8–12 minutes on four cores
the first time and well under a minute afterwards, since the build cache is
reused. If you want to overlap the wait, run `make build` first in one shell and
`make quickstart` after it finishes.

---

## Hot-reloading the operator

Two workflows, depending on whether you want the operator on the host or in a
container.

### Host process against the kind cluster (fastest inner loop)

```bash
make run-dev   # RUST_LOG=debug cargo watch -x run
```

`cargo watch` rebuilds and restarts the operator on every save. It talks to
whatever cluster your current kubeconfig context points at, so make sure that is
the kind cluster:

```bash
kubectl config use-context kind-stellar-dev
```

Scale the in-cluster operator to zero first, or the two will fight over the same
resources:

```bash
kubectl scale deploy/stellar-operator -n stellar-system --replicas=0
```

Related targets: `make run-local` runs the release binary once without
watching, and `make watch` runs `cargo watch -x check -x test -x build` when you
want continuous checking rather than a running operator.

### Containerised hot-reload

```bash
make compose-dev   # docker-compose.yml + docker-compose.dev.yml
```

This runs the operator from `Dockerfile.dev` with `cargo watch` inside the
container, mounting the source tree at `/app` with persistent cargo and target
caches, `RUST_LOG=debug,stellar_k8s=trace`, and `--dry-run` enabled. Use it when
you want an environment closer to CI. `make compose-logs` tails it and
`make compose-down` stops it.

> On WSL2 and macOS, in-container file watching only works reliably when the
> source tree lives in the Linux/VM filesystem. See the platform notes above.

---

## Integration tests against kind

Unit and doc tests need no cluster:

```bash
make test
```

The kind end-to-end suite is `#[ignore]`d by default so it never runs in a
normal `cargo test`. Run it explicitly against a live cluster:

```bash
cargo test --test e2e_kind -- --ignored --nocapture
```

The harness creates or reuses a cluster named **`stellar-e2e`**, which is
deliberately *not* the `stellar-dev` cluster `make quickstart` builds, so a test
run cannot destroy your development environment. Point it at another cluster
with:

```bash
KIND_CLUSTER_NAME=stellar-dev cargo test --test e2e_kind -- --ignored
```

The suite installs CRDs from `config/crd/`, applies sample `StellarNode`
manifests, and waits for the operator to reconcile Deployments and Services
across the `stellar-e2e`, `stellar-e2e-horizon` and `stellar-e2e-upgrade`
namespaces. It skips itself when `kind`/`kubectl` are missing rather than
failing, so a green run with no output means the tools were not found — check
that both are on `PATH`.

Other suites in `tests/` (chaos, DR failover, leader election, secret rotation,
service mesh) follow the same `--ignored` convention where they need a cluster.

Before opening a PR, run what CI runs:

```bash
make ci-local   # fmt-check + lint + audit + test + build
```

---

## Makefile shortcuts

`make help` prints the full list. The ones that matter day to day:

| Target | What it does |
| --- | --- |
| `make dev-setup` | rustup stable + clippy/rustfmt, `cargo-audit`, `cargo-watch`, pre-commit hooks |
| `make quickstart` | kind cluster, image build and load, CRD, Helm install, sample node |
| `make build` | `cargo build --release --locked` |
| `make test` | unit, integration and doc tests with the standard feature set |
| `make run-dev` | operator with hot-reload (`cargo watch`) |
| `make run-local` | run the release binary once |
| `make watch` | continuous `check` + `test` + `build` |
| `make compose-dev` | containerised hot-reload stack |
| `make install-crd` | apply `config/crd/stellarnode-crd.yaml` |
| `make apply-samples` | install CRDs, then apply everything in `config/samples/` |
| `make fmt` / `make lint` | `cargo fmt --all` / clippy |
| `make ci-local` | the full CI pipeline locally |
| `make docker-build` | local image from host-built binaries |
| `make helm-lint` | lint `charts/stellar-operator` |
| `make clean` | `cargo clean` |

---

## Diagnostics

### The cluster never becomes ready, or nodes go `NotReady`

Almost always memory. A kind node plus a Rust build easily exceeds a 2 GB Docker
VM. Check what Docker actually has:

```bash
docker info --format '{{.NCPU}} CPUs, {{.MemTotal}} bytes'
```

Raise it in Docker Desktop (Settings → Resources) or in `.wslconfig` on Windows,
then `kind delete cluster --name stellar-dev` and re-run `make quickstart`.

### `too many open files` when creating the cluster

The inotify limits. Apply the `sysctl` settings from the Linux setup section
above; they apply to WSL2 as well.

### Pods stuck in `ImagePullBackOff`

The chart is deployed with `image.pullPolicy=Never`, so the node must already
have the image. Confirm the load step happened:

```bash
docker exec -it stellar-dev-control-plane crictl images | grep stellar-operator
kind load docker-image stellar-operator:dev --name stellar-dev
```

Rebuild and reload after any code change — `kind load` copies the image, it does
not track it.

### Pods evicted, or `no space left on device`

kind stores everything in the Docker VM's disk. Reclaim it:

```bash
docker system df
docker system prune -a --volumes   # removes unused images/volumes, not your cluster
```

If the operator's PVCs fill the node, delete the sample and reapply:
`kubectl delete -f config/samples/test-stellarnode.yaml`.

### `cargo watch` does not rebuild on save

Either the inotify limits above, or the source tree is on a mounted Windows/host
filesystem where change events are not delivered. Move the clone into the Linux
filesystem (WSL2) and retry.

### The operator reconciles nothing

Check that only one operator is running. If you started `make run-dev` while the
in-cluster deployment is still up, both are watching the same resources:

```bash
kubectl get deploy/stellar-operator -n stellar-system   # scale to 0 for host-side dev
```

Also confirm the CRD is installed (`kubectl get crd | grep stellarnode`) and
your kubeconfig context is `kind-stellar-dev`.

### Helm release fails to become ready within the timeout

`make quickstart` waits 120 s. On a slow first run the image load and pod start
can exceed that. The release is still installing — watch it, then re-run
`make quickstart`, which is idempotent:

```bash
kubectl get pods -n stellar-system -w
```

---

## Teardown

```bash
kind delete cluster --name stellar-dev   # development cluster
kind delete cluster --name stellar-e2e   # e2e test cluster, if created
make clean                               # cargo artifacts
docker rmi stellar-operator:dev
```
