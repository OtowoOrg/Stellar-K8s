# Database Migration Testing & Authoring Conventions

This document defines operator-owned SQL migrations, the automated forward/rollback test harness, data integrity checks, authoring conventions, and the pull request review process for schema evolution safety (Issue #1403).

Horizon still owns its application schema. The operator applies Horizon upgrades
with `horizon db upgrade || horizon db init` via the `horizon-db-migration` init
container (`src/controller/resources.rs`). The SQL in `db/migrations/` records
and verifies those runs — it does **not** re-implement Horizon's schema.

## Migration Technology Stack

| Concern | Tool | Description |
|---------|------|-------------|
| Operator-owned SQL | Versioned `db/migrations/*.up.sql` / `*.down.sql` files executed with **sqlx** | Clean SQL scripts for audit and status tracking |
| Horizon application schema | Horizon CLI (`horizon db upgrade`) | Managed application schema evolution |
| Automated Test Harness | `tests/db_migration_harness.rs` + `src/db_migrations.rs` | Isolated test database runner for forward/rollback and integrity verification |
| Kubernetes CRD evolution | `scripts/crd_migration_lint.py` | Schema compatibility checks for CRDs |

Do not introduce external ORM migration engines (Flyway, Diesel, sqlx-migrate). The harness uses `sqlx::query`
so up/down scripts remain standard PostgreSQL SQL.

## Migration Naming & File Conventions

Files must strictly follow the naming pattern:

```text
db/migrations/NNNN_short_name.up.sql
db/migrations/NNNN_short_name.down.sql
```

1. **Version Prefix (`NNNN`)**: Monotonically increasing, 4-digit zero-padded integer (e.g., `0001`, `0002`).
2. **Name (`short_name`)**: Lowercase `snake_case` describing the schema change (e.g., `add_integrity_index`).
3. **Paired Execution**: Every `.up.sql` script **must** have a corresponding `.down.sql` script.
4. **Idempotency**: Use `IF NOT EXISTS` / `IF EXISTS` constructs so migrations can be safely re-applied in isolated schemas.

## Forward & Rollback Strategy

### Forward Migrations (`*.up.sql`)
- Must be additive and safe for existing data.
- New columns added to existing tables must either be `NULLABLE` or include a default/backfill strategy in the same script.
- Execute within transactions (`BEGIN ... COMMIT`) to ensure atomicity.

### Rollback Migrations (`*.down.sql`)
- Must cleanly revert schema changes introduced in the corresponding `up` script.
- Restores the exact database schema expected by the previous operator release version.
- Reverts column additions, table creations, and index additions without leaving orphan objects.

## Data Integrity Checks

The automated harness (`src/db_migrations.rs`) captures pre- and post-migration snapshots (`IntegritySnapshot`) and asserts the following facts:

1. **Row Count Preservation**: Seeded test rows must remain intact across forward migrations and rollbacks.
2. **Key & Constraint Preservation**: Primary keys, unique constraints, and foreign key relationships are checked before and after execution.
3. **Column Nullability & Data Transformations**: Verifies data backfills (e.g., checksum hex digests) meet strict non-null and format constraints.
4. **Index & Schema Validity**: Ensures required performance indexes exist after forward migration and are cleaned up on rollback.

## Running Automated Migration Tests

Automated migration tests execute against an isolated test database (PostgreSQL 16+):

```bash
# Provision isolated Postgres instance
export DATABASE_URL=postgres://stellar:stellar_test@127.0.0.1:5432/stellar_migration_test

# Execute harness tests
make test-db-migrations
# or
bash scripts/ci/test-db-migrations.sh
# or
cargo test --test db_migration_harness -- --nocapture
```

The harness runs two full test suites:
- **Fresh Database Suite**: Empty schema → All UP migrations → Verify schema → All DOWN migrations → Verify clean drop → Re-apply UP migrations.
- **Existing Data Suite**: Apply base V1 → Seed representative production rows → Capture pre-integrity snapshot → Apply UP migrations → Assert row preservation & checksum transformation → Rollback → Verify backward compatibility → Re-apply.

## Review & Authoring Checklist

Before opening or merging a PR containing database schema changes:

- [ ] **Paired Files**: Every `NNNN_name.up.sql` has a matching `NNNN_name.down.sql`.
- [ ] **Idempotency**: Up scripts execute cleanly on both empty databases and populated databases.
- [ ] **Data Integrity**: Non-destructive transformations keep row counts and primary keys intact.
- [ ] **Rollback Safety**: Down scripts restore a schema compatible with previous operator releases.
- [ ] **No Secret Data**: No production credentials or hardcoded secrets in SQL files.
- [ ] **CI Pass**: `scripts/ci/test-db-migrations.sh` passes cleanly in CI.

