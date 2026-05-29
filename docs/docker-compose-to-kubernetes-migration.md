# Docker Compose To Kubernetes Migration Guide

This guide shows how to migrate an existing Docker Compose based Stellar deployment to Kubernetes with the `Stellar-K8s` operator. It is written for teams currently running one or more Compose services such as `stellar-core`, `horizon`, `soroban-rpc`, and PostgreSQL, and who want an operator-managed, production-ready deployment model.

The guide is intentionally migration-focused:

- It starts with planning and inventory work before any cutover.
- It uses repo-native `StellarNode` resources instead of hand-written Deployments.
- It includes data migration, rollback, and validation procedures.
- It ships with a conversion helper at `scripts/compose_to_stellark8s.py`.
- It includes example input/output manifests under `examples/migrations/docker-compose/`.

## Scope

Use this guide when your current deployment looks like one or more of the following:

- A single Compose file running `stellar-core` with a bind mount or named volume.
- A Compose stack running `stellar-core`, `horizon`, and `postgres`.
- A Compose stack running `soroban-rpc` that points to a local or remote validator.
- A development stack that stores secrets in `.env` files or inline Compose environment blocks.

This guide does not attempt a one-click migration for every Compose feature. In particular, `depends_on`, host networking, custom sidecars, and ad hoc shell entrypoints still require manual review after conversion.

## Migration Outcomes

After migration, the Compose concerns below map to operator-managed Kubernetes resources:

| Docker Compose concept | Kubernetes / operator target |
|---|---|
| `services.stellar-core` | `StellarNode` with `spec.nodeType: Validator` |
| `services.horizon` | `StellarNode` with `spec.nodeType: Horizon` |
| `services.soroban-rpc` | `StellarNode` with `spec.nodeType: SorobanRpc` |
| bind mounts / named volumes | `spec.storage` PVCs or snapshot-based bootstrap |
| inline environment secrets | Kubernetes `Secret` or `validatorConfig.seedSecretSource` |
| `depends_on` | Kubernetes readiness, service discovery, and reconciliation |
| `ports` | ClusterIP service, optional `ingress` or `loadBalancer` |
| process restarts | operator reconciliation and rollout strategy |

## 1. Migration Planning

Do not start by rewriting YAML. Start by building a migration worksheet for the current Compose deployment.

### 1.1 Inventory the current stack

Capture the following for every Compose service:

- Service name and image tag.
- Node role: Validator, Horizon, Soroban RPC, PostgreSQL, or helper.
- Persistent data location and size.
- Environment variables and secret sources.
- Published ports and external consumers.
- Startup ordering assumptions from `depends_on`.
- Health checks and restart policies.

Recommended worksheet columns:

| Service | Role | Image | Data path | Secret inputs | External ports | Replacement |
|---|---|---|---|---|---|---|
| `stellar-core` | Validator | `stellar/stellar-core:21.0.0` | `/var/lib/stellar` | `STELLAR_CORE_SEED` | `11625`, `11626` | `StellarNode/Validator` |
| `horizon` | Horizon | `stellar/horizon:2.30.0` | none or app cache | `DATABASE_URL` | `8000` | `StellarNode/Horizon` |
| `postgres` | Database | `postgres:16` | `/var/lib/postgresql/data` | `POSTGRES_PASSWORD` | none | managed or external DB |

### 1.2 Decide the target Kubernetes model

Choose these target-state decisions before cutover:

- Namespace per environment, such as `stellar-mainnet` or `stellar-testnet`.
- Storage class for ledger data and database data.
- Secret source:
  - local Kubernetes Secret for development
  - External Secrets / Vault / CSI-backed source for production
- Exposure model:
  - internal only
  - `ingress`
  - `loadBalancer`
- Database strategy for Horizon:
  - keep existing external PostgreSQL and reference it with `horizonConfig.databaseSecretRef`
  - move to an operator-managed database with `spec.managedDatabase`

### 1.3 Define cutover goals

Write down clear migration success criteria:

- Validator reaches expected ledger progression on the target network.
- Horizon returns healthy responses and ingests from the correct validator.
- Soroban RPC can reach captive core / upstream core successfully.
- Secrets are no longer stored in Compose or `.env` files.
- Data volumes are durable and retained on deletion.
- Rollback can restore service to the original Compose deployment within the agreed RTO.

## 2. Read The Existing Compose File

The repository includes a starter converter:

```bash
python3 scripts/compose_to_stellark8s.py \
  --input docker-compose.yml \
  --output migrated/manifests.yaml \
  --namespace stellar-testnet \
  --network testnet \
  --storage-class standard \
  --emit-namespace
```

The converter:

- Detects `Validator`, `Horizon`, and `SorobanRpc` services heuristically.
- Emits starter `Secret` objects for seeds and database URLs.
- Emits starter `StellarNode` objects with storage, resources, and network policy enabled.
- Preserves the source Compose service name as an annotation for review.
- Prints manual review notes for unsupported Compose semantics.

### 2.1 What the converter handles well

- `environment` blocks as mappings or `KEY=value` lists.
- Compose `deploy.replicas`.
- Compose CPU and memory hints from `deploy.resources`.
- Common data-volume mount paths such as `/var/lib/stellar` and `/var/lib/postgresql/data`.
- Common service naming patterns such as `validator`, `stellar-core`, `horizon`, and `soroban-rpc`.

### 2.2 What still needs manual review

- `depends_on`
- host networking
- custom entrypoints and wrapper scripts
- TLS termination and ingress hostnames
- exact storage sizing
- exact history archive, quorum, and database tuning

## 3. Configuration Conversion Checklist

After running the converter, review each emitted manifest against the API reference in `docs/api-reference.md`.

### 3.1 Validator mapping

Typical Compose validator concerns should map to these operator fields:

| Compose concern | `StellarNode` field |
|---|---|
| Stellar Core service | `spec.nodeType: Validator` |
| image tag | `spec.version` |
| validator seed | `spec.validatorConfig.seedSecretRef` or `seedSecretSource` |
| ledger volume | `spec.storage` |
| history archives | `spec.validatorConfig.historyArchiveUrls` |
| resource limits | `spec.resources` |
| anti-affinity | `spec.podAntiAffinity` |

### 3.2 Horizon mapping

| Compose concern | `StellarNode` field |
|---|---|
| Horizon service | `spec.nodeType: Horizon` |
| database connection string | `spec.horizonConfig.databaseSecretRef` |
| upstream core URL | `spec.horizonConfig.stellarCoreUrl` |
| ingestion toggle | `spec.horizonConfig.enableIngest` |
| worker count | `spec.horizonConfig.ingestWorkers` |
| published API endpoint | `spec.ingress` or `spec.loadBalancer` |

### 3.3 Soroban RPC mapping

| Compose concern | `StellarNode` field |
|---|---|
| Soroban RPC service | `spec.nodeType: SorobanRpc` |
| upstream core URL | `spec.sorobanConfig.stellarCoreUrl` |
| captive core config | `spec.sorobanConfig.captiveCoreStructuredConfig` |
| scaling needs | `spec.autoscaling` |

### 3.4 Database mapping

Compose often runs PostgreSQL as an adjacent container. In Kubernetes you should choose one of these patterns:

1. Keep the database external and store the connection string in a Secret referenced by `spec.horizonConfig.databaseSecretRef`.
2. Use `spec.managedDatabase` if you want the operator ecosystem to manage PostgreSQL for you.
3. Restore existing PostgreSQL data into a managed platform database before cutover.

Avoid lifting a Compose PostgreSQL container into Kubernetes unchanged unless it is strictly temporary for migration testing.

## 4. Data Migration Procedures

Data migration is the highest-risk part of a Compose-to-Kubernetes move. Split it into validator data, Horizon database data, and secret migration.

### 4.1 Validator ledger data

Choose one of these paths:

1. Fresh sync:
   - simplest path for testnet or low-urgency environments
   - create the `StellarNode` with empty PVCs and allow it to catch up from history archives
2. Backup archive bootstrap:
   - export the existing ledger data into a compressed archive
   - host it on `https://` or `s3://`
   - configure `spec.storage.snapshotRef.backupUrl`
3. Snapshot-based bootstrap:
   - if the source data already exists in Kubernetes storage, use `restoreFromSnapshot`
   - see `docs/volume-snapshots.md`

Example archive-based bootstrap:

```yaml
spec:
  storage:
    storageClass: "ssd-premium"
    size: "500Gi"
    retentionPolicy: Retain
    snapshotRef:
      backupUrl: "s3://stellar-migration/validator-mainnet-ledger.tar.zst"
      credentialsSecretRef: "migration-backup-creds"
```

Before taking the validator copy:

- Stop write-heavy maintenance jobs.
- Record the current ledger sequence.
- Capture checksums or a file inventory for later validation.
- Keep the original Compose volume intact until cutover is accepted.

### 4.2 Horizon PostgreSQL data

For Horizon, prefer a logical export/import or managed database migration rather than a raw container copy.

Suggested sequence:

1. Freeze schema-changing maintenance.
2. Run `pg_dump` or `pg_dumpall` from the Compose deployment.
3. Restore into the target PostgreSQL endpoint.
4. Create a Kubernetes Secret containing `DATABASE_URL`.
5. Point `spec.horizonConfig.databaseSecretRef` at that Secret.
6. Validate row counts and recent ledger ingestion.

If the source database is large, run the restore ahead of cutover and keep it close to current with a planned final sync window.

### 4.3 Secret migration

Never move Compose secrets by copying `.env` files into Git.

Instead:

- Move validator seeds into Kubernetes Secrets for development only.
- For production, use `spec.validatorConfig.seedSecretSource`.
- Move database URLs, API keys, and backup credentials into Secrets.
- Rotate credentials after the migration completes.

See `docs/secret-management-kms.md` and `docs/secret-rotation.md`.

## 5. Cutover Procedure

Use a staged migration, not a direct replacement.

### 5.1 Dry-run environment

Create a dry-run namespace first:

```bash
kubectl create namespace stellar-migration
kubectl label namespace stellar-migration stellar.org/network=testnet
kubectl apply -f migrated/manifests.yaml
```

Validate:

- `StellarNode` resources reconcile successfully.
- PVCs bind to the expected storage class.
- Secrets mount correctly.
- Validators progress beyond startup.
- Horizon and Soroban can resolve upstream core services.

### 5.2 Parallel run

Before traffic cutover:

- Keep Compose running as the current production path.
- Run Kubernetes workloads in parallel.
- Compare health endpoints, ledger progression, API responses, and logs.
- Confirm new persistent volumes are stable.

### 5.3 Final cutover

Suggested sequence:

1. Announce a migration window.
2. Stop write traffic or move to read-only mode where possible.
3. Perform final validator archive export or final database sync.
4. Update Secrets with final connection details.
5. Confirm Kubernetes pods are healthy.
6. Switch DNS, ingress, or client routing to Kubernetes endpoints.
7. Watch the environment closely for at least one full validation window.

## 6. Rollback Procedures

Plan rollback before starting migration. The safest rollback is usually traffic reversion, not reverse data sync.

### 6.1 Rollback triggers

Trigger rollback if any of the following occurs:

- Validator fails to progress on the correct network.
- Horizon ingests the wrong database or wrong validator source.
- Soroban RPC cannot serve production traffic reliably.
- Secret injection fails.
- PVC performance is materially worse than the Compose baseline.

### 6.2 Rollback plan

1. Keep the original Compose deployment and volumes untouched until migration sign-off.
2. Do not destroy the original Compose database until post-cutover validation completes.
3. Repoint DNS or ingress back to Compose endpoints.
4. Scale Kubernetes-facing traffic to zero or remove external exposure.
5. Preserve Kubernetes logs, events, and manifests for analysis.
6. Record the exact cutover and rollback timestamps.

### 6.3 Rollback data considerations

- If Kubernetes accepted new write traffic, decide whether that data must be replayed into the Compose environment before rollback.
- If replay is not feasible safely, prefer a short maintenance window over split-brain writes.
- Never run two active writers against the same Horizon database without a deliberate replication design.

## 7. Testing Checklist

Use this checklist before declaring the migration complete.

```text
[ ] Namespace created and labelled with stellar.org/network
[ ] Secrets exist and do not contain placeholder values
[ ] PVCs are bound to the expected storage class
[ ] Validator reaches expected ledger progression
[ ] Horizon can connect to its database and upstream core
[ ] Soroban RPC can reach upstream core and serve requests
[ ] Network policies allow required traffic and block unintended paths
[ ] Ingress or service exposure matches the target design
[ ] Old Compose endpoints remain available for rollback until sign-off
[ ] Backup / restore evidence captured for migrated data
[ ] Runbooks updated with new Kubernetes endpoints and commands
```

### 7.1 Functional validation commands

Example checks:

```bash
kubectl get stellarnodes -n stellar-testnet
kubectl describe stellarnode validator-testnet -n stellar-testnet
kubectl get pods,pvc,svc -n stellar-testnet
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator --tail=100
```

Review:

- `status.phase`
- readiness conditions
- `status.ledgerSequence`
- `status.ledgerUpdatedAt`
- service reachability from peer workloads

### 7.2 Data validation checks

For validator data:

- compare the last known source ledger sequence to the recovered target
- compare file counts or checksums if an archive migration was used

For Horizon data:

- compare row counts on high-value tables
- compare latest ledger and transaction records
- compare representative API responses before and after cutover

## 8. Common Pitfalls

These are the most common migration mistakes when moving from Compose to the operator.

### 8.1 Treating `depends_on` as readiness

Compose startup ordering is not equivalent to Kubernetes readiness. Replace it with:

- correct service URLs
- readiness probes
- patience for reconciliation and startup

### 8.2 Copying inline secrets into manifests

Do not translate Compose environment variables directly into plain-text CRD fields if the value is sensitive. Use Secrets or `seedSecretSource`.

### 8.3 Under-sizing storage

Compose bind mounts often hide real data growth. Explicitly size Kubernetes PVCs based on:

- current used bytes
- expected annual growth
- restore scratch space

Use `Retain` during migration windows for safer rollback.

### 8.4 Forgetting namespace isolation

If you move mainnet and testnet workloads into a shared namespace, you lose an important safety boundary. Follow `docs/network-isolation.md`.

### 8.5 Migrating PostgreSQL as if it were stateless

Do not treat Horizon database data like application config. Use a deliberate database migration method and validate it independently.

### 8.6 Assuming Compose ports map directly to Kubernetes exposure

Published host ports in Compose do not automatically become production-safe Kubernetes exposure. Review `ingress`, `loadBalancer`, TLS, and authentication separately.

## 9. Example Migrations

The repository includes example artifacts:

- Input Compose file: `examples/migrations/docker-compose/docker-compose.validator-horizon.yml`
- Converted output: `examples/migrations/docker-compose/converted-stellarnodes.yaml`

### 9.1 Example A: Validator plus Horizon plus PostgreSQL

Source pattern:

- one `stellar-core` validator
- one `horizon` API service
- one `postgres` service

Target pattern:

- one Validator `StellarNode`
- one Horizon `StellarNode`
- one Secret for validator seed
- one Secret for `DATABASE_URL`
- PVC-backed storage for validator data

### 9.2 Example B: Single validator development stack

Source pattern:

- one Compose validator service
- seed in `.env`
- bind mount for ledger data

Target pattern:

- one Validator `StellarNode`
- one local Kubernetes Secret for the seed
- one retained PVC

## 10. Video Tutorial Asset

This repository includes a video-ready tutorial script at `docs/docker-compose-migration-video-tutorial.md`.

Use it to record:

- a short planning walkthrough
- a live converter demo
- a manifest review
- a dry-run deployment demo
- cutover and rollback discussion

## 11. Recommended Migration Sequence

If you want the shortest safe path, use this order:

1. inventory the Compose stack
2. choose namespace, storage, and secret strategy
3. run the converter
4. review and harden the generated manifests
5. migrate secrets
6. migrate validator and database data
7. validate in a dry-run namespace
8. run Compose and Kubernetes in parallel
9. cut over traffic
10. hold the old stack for rollback until acceptance completes

## References

- `scripts/compose_to_stellark8s.py`
- `docs/api-reference.md`
- `docs/network-isolation.md`
- `docs/volume-snapshots.md`
- `docs/backup-verification.md`
- `docs/secret-management-kms.md`
- `docs/secret-rotation.md`
- `examples/validator-mainnet.yaml`
- `examples/validator-testnet.yaml`
- `examples/horizon.yaml`
- `examples/soroban-rpc.yaml`
