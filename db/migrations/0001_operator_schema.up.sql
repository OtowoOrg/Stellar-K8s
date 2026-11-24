-- Operator-owned schema used to record Horizon/Soroban database migration runs.
-- Horizon still owns its application schema via `horizon db upgrade`.
-- These tables let the operator prove forward/rollback safety independently.

CREATE TABLE IF NOT EXISTS operator_schema_migrations (
    version     INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS horizon_migration_runs (
    id              BIGSERIAL PRIMARY KEY,
    node_name       TEXT NOT NULL,
    namespace       TEXT NOT NULL,
    horizon_version TEXT NOT NULL,
    direction       TEXT NOT NULL CHECK (direction IN ('up', 'down')),
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at     TIMESTAMPTZ,
    success         BOOLEAN NOT NULL DEFAULT FALSE,
    row_count_before BIGINT,
    row_count_after  BIGINT
);

CREATE INDEX IF NOT EXISTS horizon_migration_runs_node_idx
    ON horizon_migration_runs (namespace, node_name);
