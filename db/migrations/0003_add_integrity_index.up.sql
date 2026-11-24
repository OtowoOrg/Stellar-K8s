-- Speeds up integrity lookups used by the migration harness and operator audits.

CREATE UNIQUE INDEX IF NOT EXISTS horizon_migration_runs_checksum_uidx
    ON horizon_migration_runs (checksum);

CREATE TABLE IF NOT EXISTS horizon_migration_invariants (
    run_id          BIGINT NOT NULL REFERENCES horizon_migration_runs (id) ON DELETE CASCADE,
    table_name      TEXT NOT NULL,
    row_count       BIGINT NOT NULL,
    captured_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (run_id, table_name)
);
