-- Adds an integrity checksum column and backfills existing rows.
-- The backfill is the intentional data transformation covered by the harness.

ALTER TABLE horizon_migration_runs
    ADD COLUMN IF NOT EXISTS checksum TEXT;

UPDATE horizon_migration_runs
SET checksum = md5(
    concat_ws(
        ':',
        id::text,
        node_name,
        namespace,
        horizon_version,
        direction
    )
)
WHERE checksum IS NULL;

ALTER TABLE horizon_migration_runs
    ALTER COLUMN checksum SET NOT NULL;
