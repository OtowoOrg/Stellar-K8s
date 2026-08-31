// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Operator-owned SQL migration runner and integrity harness.
//!
//! Horizon still applies its own schema with `horizon db upgrade` (see
//! [`crate::controller::resources::build_horizon_migration_container`]). This
//! module versions the operator-owned audit tables that record those runs and
//! provides the automated forward / rollback / integrity checks required by
//! issue #1317.
//!
//! Migrations live in `db/migrations/` as paired `*.up.sql` / `*.down.sql`
//! files and are executed with the existing `sqlx` PostgreSQL client.

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A single versioned SQL migration with optional down script.
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: u32,
    pub name: String,
    pub up_sql: String,
    pub down_sql: String,
}

/// Snapshot of integrity-relevant state captured around a migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegritySnapshot {
    pub schema: String,
    pub tables: BTreeMap<String, TableSnapshot>,
}

/// Per-table integrity facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSnapshot {
    pub row_count: i64,
    pub primary_keys: Vec<String>,
    pub columns: Vec<ColumnSnapshot>,
    pub indexes: Vec<String>,
    pub foreign_keys: Vec<String>,
}

/// Column nullability and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSnapshot {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

/// Load versioned SQL files from `db/migrations`.
pub fn load_migrations(dir: impl AsRef<Path>) -> Result<Vec<Migration>, String> {
    let dir = dir.as_ref();
    if !dir.is_dir() {
        return Err(format!("migration directory not found: {}", dir.display()));
    }

    let mut ups: BTreeMap<u32, (String, String)> = BTreeMap::new();
    let mut downs: BTreeMap<u32, String> = BTreeMap::new();

    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("failed to read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sql") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("invalid migration file name: {}", path.display()))?;
        let (version, name, kind) = parse_migration_filename(file_name)?;
        let sql = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        match kind {
            "up" => {
                ups.insert(version, (name, sql));
            }
            "down" => {
                downs.insert(version, sql);
            }
            other => return Err(format!("unknown migration kind '{other}' in {file_name}")),
        }
    }

    if ups.is_empty() {
        return Err("no up migrations found".into());
    }

    let mut migrations = Vec::new();
    for (version, (name, up_sql)) in ups {
        let down_sql = downs.remove(&version).ok_or_else(|| {
            format!("migration {version:04}_{name} is missing a matching .down.sql")
        })?;
        migrations.push(Migration {
            version,
            name,
            up_sql,
            down_sql,
        });
    }
    Ok(migrations)
}

fn parse_migration_filename(file_name: &str) -> Result<(u32, String, &str), String> {
    // 0001_operator_schema.up.sql
    let stem = file_name
        .strip_suffix(".sql")
        .ok_or_else(|| format!("expected .sql suffix: {file_name}"))?;
    let (prefix, kind) = stem
        .rsplit_once('.')
        .ok_or_else(|| format!("expected .up.sql or .down.sql: {file_name}"))?;
    let (version_str, name) = prefix
        .split_once('_')
        .ok_or_else(|| format!("expected NNNN_name.up.sql: {file_name}"))?;
    let version: u32 = version_str
        .parse()
        .map_err(|_| format!("invalid version prefix in {file_name}"))?;
    if name.is_empty() {
        return Err(format!("empty migration name in {file_name}"));
    }
    Ok((version, name.to_string(), kind))
}

/// Default on-disk location relative to the crate root.
pub fn default_migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("db/migrations")
}

/// Connect to Postgres with a short timeout suitable for tests.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
}

/// Connect with `search_path` pinned so every pooled client sees the same schema.
pub async fn connect_with_schema(database_url: &str, schema: &str) -> Result<PgPool, sqlx::Error> {
    let schema = schema.to_string();
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(move |conn, _meta| {
            let schema = schema.clone();
            Box::pin(async move {
                let quoted = quote_ident(&schema);
                sqlx::query(&format!("SET search_path TO {quoted}, public"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
}

/// Create an isolated schema, run `body`, then drop the schema.
pub async fn with_temp_schema<F, Fut, T>(
    database_url: &str,
    schema: &str,
    body: F,
) -> Result<T, String>
where
    F: FnOnce(PgPool) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let admin = connect(database_url)
        .await
        .map_err(|e| format!("admin connect: {e}"))?;
    let quoted = quote_ident(schema);
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {quoted} CASCADE"))
        .execute(&admin)
        .await
        .map_err(|e| format!("drop leftover schema {schema}: {e}"))?;
    sqlx::query(&format!("CREATE SCHEMA {quoted}"))
        .execute(&admin)
        .await
        .map_err(|e| format!("create schema {schema}: {e}"))?;

    let pool = connect_with_schema(database_url, schema)
        .await
        .map_err(|e| format!("schema connect: {e}"))?;
    let result = body(pool.clone()).await;
    pool.close().await;

    let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {quoted} CASCADE"))
        .execute(&admin)
        .await;
    admin.close().await;
    result
}

/// Apply every up migration in order and record versions.
pub async fn migrate_up(pool: &PgPool, migrations: &[Migration]) -> Result<(), String> {
    for migration in migrations {
        exec_sql(pool, &migration.up_sql)
            .await
            .map_err(|e| format!("up {}_{}: {e}", migration.version, migration.name))?;
        record_version(pool, migration).await?;
    }
    Ok(())
}

/// Roll back migrations in reverse, stopping after `count` downs (or all).
pub async fn migrate_down(
    pool: &PgPool,
    migrations: &[Migration],
    count: Option<usize>,
) -> Result<(), String> {
    let take = count.unwrap_or(migrations.len());
    for migration in migrations.iter().rev().take(take) {
        exec_sql(pool, &migration.down_sql)
            .await
            .map_err(|e| format!("down {}_{}: {e}", migration.version, migration.name))?;
        if table_exists(pool, "operator_schema_migrations").await? {
            sqlx::query("DELETE FROM operator_schema_migrations WHERE version = $1")
                .bind(migration.version as i32)
                .execute(pool)
                .await
                .map_err(|e| format!("delete version {}: {e}", migration.version))?;
        }
    }
    Ok(())
}

async fn record_version(pool: &PgPool, migration: &Migration) -> Result<(), String> {
    if !table_exists(pool, "operator_schema_migrations").await? {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO operator_schema_migrations (version, name)
         VALUES ($1, $2)
         ON CONFLICT (version) DO NOTHING",
    )
    .bind(migration.version as i32)
    .bind(&migration.name)
    .execute(pool)
    .await
    .map_err(|e| format!("record version {}: {e}", migration.version))?;
    Ok(())
}

/// Seed representative pre-migration rows used by the existing-database path.
pub async fn seed_representative_data(pool: &PgPool) -> Result<Vec<i64>, String> {
    let mut ids = Vec::new();
    for (node, ns, version, direction) in [
        ("horizon-a", "stellar-system", "2.30.0", "up"),
        ("horizon-b", "stellar-system", "2.30.0", "up"),
        ("horizon-a", "payments", "2.31.0", "down"),
    ] {
        let row = sqlx::query(
            "INSERT INTO horizon_migration_runs
                (node_name, namespace, horizon_version, direction, success, row_count_before, row_count_after)
             VALUES ($1, $2, $3, $4, TRUE, 10, 10)
             RETURNING id",
        )
        .bind(node)
        .bind(ns)
        .bind(version)
        .bind(direction)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("seed insert: {e}"))?;
        ids.push(row.get::<i64, _>("id"));
    }
    Ok(ids)
}

/// Capture table counts, PKs, columns, indexes, and foreign keys.
pub async fn capture_integrity(pool: &PgPool, schema: &str) -> Result<IntegritySnapshot, String> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname = $1 ORDER BY tablename",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("list tables: {e}"))?;

    let mut snap = IntegritySnapshot {
        schema: schema.to_string(),
        tables: BTreeMap::new(),
    };

    for table in tables {
        let row_count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {}.{}",
            quote_ident(schema),
            quote_ident(&table)
        ))
        .fetch_one(pool)
        .await
        .map_err(|e| format!("count {table}: {e}"))?;

        let columns = sqlx::query(
            "SELECT column_name, data_type, is_nullable
             FROM information_schema.columns
             WHERE table_schema = $1 AND table_name = $2
             ORDER BY ordinal_position",
        )
        .bind(schema)
        .bind(&table)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("columns {table}: {e}"))?;

        let columns: Vec<ColumnSnapshot> = columns
            .into_iter()
            .map(|row| ColumnSnapshot {
                name: row.get("column_name"),
                data_type: row.get("data_type"),
                nullable: row.get::<String, _>("is_nullable") == "YES",
            })
            .collect();

        let primary_keys: Vec<String> = sqlx::query_scalar(
            "SELECT a.attname
             FROM pg_index i
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
             JOIN pg_class c ON c.oid = i.indrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE i.indisprimary AND n.nspname = $1 AND c.relname = $2
             ORDER BY a.attname",
        )
        .bind(schema)
        .bind(&table)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("pks {table}: {e}"))?;

        let indexes: Vec<String> = sqlx::query_scalar(
            "SELECT indexname FROM pg_indexes
             WHERE schemaname = $1 AND tablename = $2
             ORDER BY indexname",
        )
        .bind(schema)
        .bind(&table)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("indexes {table}: {e}"))?;

        let foreign_keys: Vec<String> = sqlx::query_scalar(
            "SELECT con.conname
             FROM pg_constraint con
             JOIN pg_class rel ON rel.oid = con.conrelid
             JOIN pg_namespace nsp ON nsp.oid = rel.relnamespace
             WHERE con.contype = 'f' AND nsp.nspname = $1 AND rel.relname = $2
             ORDER BY con.conname",
        )
        .bind(schema)
        .bind(&table)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("fks {table}: {e}"))?;

        snap.tables.insert(
            table,
            TableSnapshot {
                row_count,
                primary_keys,
                columns,
                indexes,
                foreign_keys,
            },
        );
    }
    Ok(snap)
}

/// Assert seeded run identities survived a later migration.
pub async fn assert_seed_rows_intact(pool: &PgPool, ids: &[i64]) -> Result<(), String> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM horizon_migration_runs WHERE id = ANY($1)")
            .bind(ids)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("seed lookup: {e}"))?;
    if count as usize != ids.len() {
        return Err(format!(
            "expected {} seeded runs to remain, found {count}",
            ids.len()
        ));
    }
    Ok(())
}

/// Assert migration 0002's checksum backfill produced a stable non-null hash.
pub async fn assert_checksum_transformation(pool: &PgPool, ids: &[i64]) -> Result<(), String> {
    let rows = sqlx::query(
        "SELECT id, checksum FROM horizon_migration_runs WHERE id = ANY($1) ORDER BY id",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("checksum lookup: {e}"))?;
    if rows.len() != ids.len() {
        return Err("checksum transformation dropped seeded rows".into());
    }
    for row in rows {
        let checksum: String = row.get("checksum");
        if checksum.len() != 32 || !checksum.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("invalid checksum '{checksum}'"));
        }
    }
    Ok(())
}

/// Verify the operator schema after all forward migrations.
pub async fn assert_forward_schema(pool: &PgPool, schema: &str) -> Result<(), String> {
    let snap = capture_integrity(pool, schema).await?;
    for required in [
        "operator_schema_migrations",
        "horizon_migration_runs",
        "horizon_migration_invariants",
    ] {
        snap.tables
            .get(required)
            .ok_or_else(|| format!("missing table {required} after forward migrations"))?;
    }

    let runs = snap
        .tables
        .get("horizon_migration_runs")
        .expect("checked above");
    let checksum = runs
        .columns
        .iter()
        .find(|c| c.name == "checksum")
        .ok_or_else(|| "horizon_migration_runs.checksum missing".to_string())?;
    if checksum.nullable {
        return Err("checksum must be NOT NULL after 0002".into());
    }
    if !runs.indexes.iter().any(|i| i.contains("checksum")) {
        return Err("checksum unique index missing after 0003".into());
    }

    let invariants = snap
        .tables
        .get("horizon_migration_invariants")
        .expect("checked above");
    if invariants.foreign_keys.is_empty() {
        return Err("horizon_migration_invariants is missing its foreign key".into());
    }
    Ok(())
}

/// Verify the schema after rolling back to version 1 (pre-checksum).
pub async fn assert_post_rollback_v1(pool: &PgPool, schema: &str) -> Result<(), String> {
    let snap = capture_integrity(pool, schema).await?;
    if snap.tables.contains_key("horizon_migration_invariants") {
        return Err("horizon_migration_invariants should not exist after rollback to v1".into());
    }
    let runs = snap
        .tables
        .get("horizon_migration_runs")
        .ok_or_else(|| "horizon_migration_runs missing after rollback".to_string())?;
    if runs.columns.iter().any(|c| c.name == "checksum") {
        return Err("checksum column should be removed by 0002 down".into());
    }
    snap.tables
        .get("operator_schema_migrations")
        .ok_or_else(|| "operator_schema_migrations missing after rollback to v1".to_string())?;
    Ok(())
}

/// Full fresh-database path: empty schema → all ups → integrity → all downs → re-apply.
pub async fn run_fresh_path(
    pool: &PgPool,
    schema: &str,
    migrations: &[Migration],
) -> Result<(), String> {
    migrate_up(pool, migrations).await?;
    assert_forward_schema(pool, schema).await?;
    let versions = applied_versions(pool).await?;
    if versions.len() != migrations.len() {
        return Err(format!(
            "expected {} applied versions, found {}",
            migrations.len(),
            versions.len()
        ));
    }
    migrate_down(pool, migrations, None).await?;
    let snap = capture_integrity(pool, schema).await?;
    if snap.tables.contains_key("horizon_migration_runs")
        || snap.tables.contains_key("operator_schema_migrations")
    {
        return Err("tables remained after full rollback".into());
    }
    migrate_up(pool, migrations).await?;
    assert_forward_schema(pool, schema).await
}

/// Existing-database path: apply v1, seed, capture, remaining ups, verify data + transform.
pub async fn run_existing_data_path(
    pool: &PgPool,
    schema: &str,
    migrations: &[Migration],
) -> Result<(), String> {
    let (first, rest) = migrations
        .split_first()
        .ok_or_else(|| "no migrations to apply".to_string())?;
    migrate_up(pool, std::slice::from_ref(first)).await?;
    let ids = seed_representative_data(pool).await?;
    let before = capture_integrity(pool, schema).await?;
    let before_count = before
        .tables
        .get("horizon_migration_runs")
        .map(|t| t.row_count)
        .unwrap_or(0);
    if before_count != ids.len() as i64 {
        return Err(format!(
            "seed row count mismatch: expected {}, got {before_count}",
            ids.len()
        ));
    }

    migrate_up(pool, rest).await?;
    assert_seed_rows_intact(pool, &ids).await?;
    assert_checksum_transformation(pool, &ids).await?;
    assert_forward_schema(pool, schema).await?;

    let after = capture_integrity(pool, schema).await?;
    let after_count = after
        .tables
        .get("horizon_migration_runs")
        .map(|t| t.row_count)
        .unwrap_or(0);
    if after_count != before_count {
        return Err(format!(
            "row count changed unexpectedly: before={before_count} after={after_count}"
        ));
    }

    // Roll back the last two migrations (checksum + integrity index) and confirm
    // seeded identities still exist in the v1-compatible schema.
    migrate_down(pool, rest, None).await?;
    assert_post_rollback_v1(pool, schema).await?;
    assert_seed_rows_intact(pool, &ids).await?;

    migrate_up(pool, rest).await?;
    assert_checksum_transformation(pool, &ids).await?;
    assert_forward_schema(pool, schema).await
}

async fn applied_versions(pool: &PgPool) -> Result<Vec<i32>, String> {
    sqlx::query_scalar("SELECT version FROM operator_schema_migrations ORDER BY version")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("applied versions: {e}"))
}

async fn table_exists(pool: &PgPool, table: &str) -> Result<bool, String> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_schema = ANY (current_schemas(false))
              AND table_name = $1
        )",
    )
    .bind(table)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("table_exists {table}: {e}"))
}

async fn exec_sql(pool: &PgPool, sql: &str) -> Result<(), sqlx::Error> {
    for stmt in split_sql_statements(sql) {
        if stmt.is_empty() {
            continue;
        }
        sqlx::query(&stmt).execute(pool).await?;
    }
    Ok(())
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut current = String::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            continue;
        }
        current.push_str(line);
        current.push('\n');
        if trimmed.ends_with(';') {
            let stmt = current.trim().trim_end_matches(';').trim().to_string();
            if !stmt.is_empty() {
                stmts.push(stmt);
            }
            current.clear();
        }
    }
    let tail = current.trim().trim_end_matches(';').trim().to_string();
    if !tail.is_empty() {
        stmts.push(tail);
    }
    stmts
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_migration_filenames() {
        let (v, name, kind) = parse_migration_filename("0002_add_run_checksum.up.sql").unwrap();
        assert_eq!(v, 2);
        assert_eq!(name, "add_run_checksum");
        assert_eq!(kind, "up");
    }

    #[test]
    fn bundled_migrations_are_paired() {
        let migrations = load_migrations(default_migrations_dir()).expect("load migrations");
        assert!(
            migrations.len() >= 3,
            "expected at least three bundled migrations, got {}",
            migrations.len()
        );
        for (idx, migration) in migrations.iter().enumerate() {
            assert_eq!(migration.version, (idx + 1) as u32);
            assert!(!migration.up_sql.trim().is_empty());
            assert!(!migration.down_sql.trim().is_empty());
        }
    }

    #[test]
    fn split_sql_skips_comments() {
        let stmts = split_sql_statements("-- comment\nCREATE TABLE t (id INT);\nDROP TABLE t;");
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE"));
    }
}
