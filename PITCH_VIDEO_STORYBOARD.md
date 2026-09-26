# Stellar-K8s Pitch Demo — Command Sheet

**Project:** [Stellar-K8s](https://github.com/stellar/stellar-k8s) — Kubernetes operator for Stellar Core validators and Horizon/RPC nodes

**Prereq:** SSH into EC2
```bash
ssh -i ~/Documents/Documents/Projects-2026/test-lab/terraform/my-key-pair.pem ec2-user@54.88.233.191
newgrp docker   # run in every SSH session for docker/kind permissions
```

> If SSH times out, your public IP changed. Re-check and update the SG:
> `curl -s https://checkip.amazonaws.com` → allow that /32 on port 22 for `sg-0ecc858a55f64a530` (us-east-1).

---

## Demo sequence (environment is pre-staged)

### 1. Show the cluster
```bash
kind get clusters
kubectl get nodes
kubectl get pods -A
```
Expected: cluster `pitch` Ready, `stellar-operator` Running, cert-manager Running.

### 2. Load built operator image into kind (after any rebuild)
```bash
kind load docker-image stellar-operator:latest --name pitch
```

### 3. Deploy operator via Helm (if re-installing)
```bash
helm upgrade --install stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --set webhook.enabled=false \
  --set sidecar.enabled=false \
  --set certManager.enabled=true \
  --set serviceAccount.create=false \
  --set serviceAccount.name=default \
  --set image.repository=stellar-operator \
  --set image.tag=latest \
  --set image.pullPolicy=Never \
  --wait --timeout 180s
```
Notes:
- `sidecar.enabled=false`: crash-analyzer image (`ghcr.io/stellar/stellar-k8s`) is private (403).
- `serviceAccount.create=false`: pre-install hook needs the SA to already exist.
- Helm release must show `deployed`: `helm list -n stellar-system`

### 4. Apply demo validator
```bash
kubectl apply -f ~/Stellar-K8s/demo-validator.yaml
```
Uses `storageClass: local-path`, 10Gi, testnet, `seedSecretRef: validator-seed`,
`version: "21"` (real Docker Hub tag), health-sidecar image `stellar/stellar-k8s:21`
(retagged locally from the operator image — the docker.io repo does not exist).

### 5. Watch reconciliation go live
```bash
kubectl get stellarnodes -n stellar -w
kubectl get pods,pvc -n stellar
```
Expected: PVC `Bound` in seconds, pod `demo-validator-0` → `2/2 Running` within ~1 min,
StellarNode phase → `DR_Active`, operator log: `reconcile phases: ... succeeded`.
Note: the readiness probe stays **not Ready** until stellar-core finishes catching up
with testnet — show `2/2 Running` + `kubectl logs demo-validator-0 -c stellar-node`
(streaming ledgers) as the "it's alive" moment. With current network (peer ports blocked
by cloud firewall), core state shows `Joining SCP` with 0 peers — still `DR_Active`.

### 6. Operator observability commands
```bash
# Live status of the node
kubectl get stellarnode demo-validator -n stellar -o jsonpath="{.status.phase} ready={.status.readyReplicas}"
echo

# Operator logs
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator --tail=20

# Core info (testnet config verified)
kubectl exec -n stellar demo-validator-0 -c stellar-node -- wget -qO- http://localhost:11626/info | python3 -m json.tool
```

### 7. Cleanup (when done)
```bash
kind delete cluster --name pitch
```

---

## Pre-staged environment status

| Item | Value |
|---|---|
| EC2 | `i-04ce12a6d27fe8498` · c5.2xlarge · **54.88.233.191** · us-east-1 |
| SSH key | `~/Documents/Documents/Projects-2026/test-lab/terraform/my-key-pair.pem` |
| Kind cluster | `pitch` |
| Operator image | `stellar-operator:latest` (built on EC2, imagePullPolicy Never) |
| Helm release | `stellar-operator` (chart 1.5.0, ns `stellar-system`, status `deployed`) |
| CRDs | 12 installed (stellarfederation one always errors — ignorable) |
| Namespaces | `stellar-system`, `stellar` |
| Secrets | `validator-seed` (ns `stellar`) |
| Storage | `local-path` provisioner + node labels `topology.kubernetes.io/region=us-east-1, zone=us-east-1c` |
| RBAC | default SA bound to cluster-admin (demo shortcut) |
| Demo manifest | `~/Stellar-K8s/demo-validator.yaml` |

---

## One-time setup (already done — reference only)

```bash
# bootstrap (user-data script fails on curl-minimal conflict)
sudo dnf install -y --allowerasing docker git jq unzip tar gzip make

# cert-manager (required by chart CRDs/webhooks)
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.crds.yaml
helm install cert-manager cert-manager --repo https://charts.jetstack.io \
  --namespace cert-manager --create-namespace --set installCRDs=true --wait

# build operator image
cd ~/Stellar-K8s && make docker-build-ci   # ~8 min, run detached: nohup make docker-build-ci > /tmp/build.log 2>&1 &
kind load docker-image stellar-operator:latest --name pitch
```

## Known fixes applied to this environment (repo bugs found during setup)

1. **Duplicate `STELLAR_CORE_SEED` env** — legacy `seed_secret_ref` + seed injection both
   injected the seed env var → API rejected the StatefulSet. Fixed in
   `src/controller/resources.rs` (env dedupe).
2. **AppArmor pod rejection** — operator hardcoded
   `container.apparmor.security.beta.kubernetes.io/*: runtime/default`, and the EC2/kind
   host has no AppArmor → kubelet rejected every pod (`Pod was rejected: Cannot enforce
   AppArmor`), causing an infinite pod create/delete loop. Fixed: annotations now opt-in
   via `STELLAR_APPARMOR_ENABLED=true`.
3. **CRD missing rotation status fields** — `observedSeedSecretVersion` /
   `lastSecretRotationTime` were missing from the CRD schema, so the API pruned them and
   the seed-rotation watcher restarted pods forever. Patched live on the cluster CRD
   (`kubectl replace`); **still needs a fix in `charts/stellar-operator/templates/crd.yaml`**.
4. **crash-analyzer sidecar** can't pull (GHCR 403) — disable with `sidecar.enabled=false`.
5. **Helm pre-install hook** requires an existing ServiceAccount — install with
   `serviceAccount.create=false --serviceAccount.name=default`.
6. **Nonexistent image tags** — `stellar/stellar-core:v21.0.0` (404) and
   `stellar/stellar-k8s:*` (repo does not exist on docker.io) are referenced by the
   operator's health-sidecar image derivation (`resources.rs`) and repo examples.
   Workaround: `version: "21"` for core + `docker tag stellar-operator:latest
   stellar/stellar-k8s:21 && kind load docker-image stellar/stellar-k8s:21 --name pitch`.
   **Still needs a fix in the repo** (derive sidecar image from `ghcr.io/...`).
7. **Missing container command** — official `stellar/stellar-core` images have an
   empty `Cmd`, and the operator set no `command`, so the pod exited printing usage
   text. Fixed in `build_container` (`resources.rs`): validators now run
   `/usr/bin/stellar-core run --conf /config/stellar-core.cfg`.
8. **Generated config rejected by stellar-core ≥21** — two problems in
   `build_config_map` (`resources.rs`): (a) `DEPRECATED_SQL_LEDGER_STATE` (required by
   core 21+ since BucketListDB became default) was never emitted; (b) operator-managed
   keys (`CATCHUP_*`, mTLS, the new flag) were appended **after** the user's
   `quorumSet` TOML — once `[[VALIDATORS]]`/`[[HOME_DOMAINS]]` sections open, every
   later `KEY=VALUE` line is scoped into that table instead of the config root, so
   core never saw them (`Invalid configuration: DEPRECATED_SQL_LEDGER_STATE not set`).
   Fixed: operator keys are now written **first**, user `quorumSet` content last.
9. **Seed secret key name** — the operator reads key `STELLAR_CORE_SEED`; the demo
   secret only had `SEED` (placeholder). Added a valid-format testnet seed under
   `STELLAR_CORE_SEED` in secret `stellar/validator-seed`.
10. **`network: testnet` doesn't generate a usable core config** — no passphrase,
    peers, quorum, database path, or seed reach `stellar-core.cfg` unless the user
    supplies `validatorConfig.quorumSet`. The demo manifest now embeds the official
    SDF testnet config (validators, history archives, `KNOWN_PEERS`, sqlite path on
    the `/opt/stellar/data` volume) in `quorumSet`. **Worth a repo fix**: defaults
    for testnet should be generated from `spec.network`.

Rebuild after code fixes (runs detached, ~8-10 min):
```bash
nohup /tmp/build3.sh > /tmp/build3.out 2>&1 &   # build + kind load + rollout + pod restart
tail -f /tmp/build.log                            # ends with "DONE"
```
