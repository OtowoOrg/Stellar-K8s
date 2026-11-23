// Copyright 2026 Stellar-K8s Contributors
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
//! Automated database migration safety gates for the deploy pipeline
//! (issue #1507).
//!
//! Every schema migration is analyzed before merge by three gates:
//!
//! 1. [`Gate::LockRisk`] — flags statements that take long-lived table locks
//!    (`DROP COLUMN`, non-concurrent index builds, type rewrites, …).
//! 2. [`Gate::BackwardCompatibility`] — checks the migration against the live
//!    production schema snapshot so breaking changes are caught pre-merge.
//! 3. [`Gate::Rollback`] — requires a real rollback script to exist and to
//!    undo the tables the forward migration touches.
//!
//! [`run_gates`] executes all three and [`GateReport::to_junit_xml`] renders
//! a JUnit-style report, so results surface in existing PR checks with no new
//! UI. Analysis is pure string inspection — no database connection — keeping
//! gate runtime far under the 60s budget for typical migrations.
//!
//! # Example
//!
//! ```
//! use stellar_k8s::migration_safety::{MigrationFile, ProductionSchema, run_gates};
//!
//! let migration = MigrationFile {
//!     version: 7,
//!     name: "add_ledger_index".to_string(),
//!     up_sql: "CREATE INDEX CONCURRENTLY idx_ledger ON ledger (seq);".to_string(),
//!     down_sql: "DROP INDEX idx_ledger;".to_string(),
//! };
//! let report = run_gates(&migration, &ProductionSchema::default());
//! assert!(report.passed());
//! println!("{}", report.to_junit_xml());
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

// ─────────────────────────────────────────────────────────────────────────────
// Inputs
// ─────────────────────────────────────────────────────────────────────────────

/// One versioned migration under review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    /// Schema version, e.g. `7`.
    pub version: u32,
    /// Human-readable name, e.g. `add_ledger_index`.
    pub name: String,
    /// Forward DDL/DML to apply.
    pub up_sql: String,
    /// Rollback script that must undo the forward migration.
    pub down_sql: String,
}

/// A known production column used for compatibility checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProdColumn {
    /// Column name.
    pub name: String,
    /// Declared type, lowercased (e.g. `bigint`, `text`).
    pub data_type: String,
    /// Whether the column is nullable.
    pub nullable: bool,
}

/// Minimal live-production schema snapshot: table → columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProductionSchema {
    /// Tables keyed by name, each with its known columns.
    pub tables: BTreeMap<String, Vec<ProdColumn>>,
}

impl ProductionSchema {
    /// An empty snapshot (no production tables known).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `table` with its `columns`.
    pub fn with_table(mut self, table: &str, columns: Vec<ProdColumn>) -> Self {
        self.tables.insert(table.to_string(), columns);
        self
    }
}

/// Helper to build a [`ProdColumn`] in tests and callers.
pub fn prod_column(name: &str, data_type: &str, nullable: bool) -> ProdColumn {
    ProdColumn {
        name: name.to_string(),
        data_type: data_type.to_lowercase(),
        nullable,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Gates
// ─────────────────────────────────────────────────────────────────────────────

/// The three safety gates applied to every migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gate {
    /// Table-lock duration risk analysis.
    LockRisk,
    /// Backward compatibility against the live production schema.
    BackwardCompatibility,
    /// Rollback migration existence and plausibility.
    Rollback,
}

impl fmt::Display for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Gate::LockRisk => "lock-risk",
            Gate::BackwardCompatibility => "backward-compatibility",
            Gate::Rollback => "rollback",
        })
    }
}

/// Outcome of a single gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    /// Which gate ran.
    pub gate: Gate,
    /// Whether the migration may proceed.
    pub passed: bool,
    /// Human-readable explanation, including the offending statement.
    pub details: String,
}

impl GateResult {
    /// A passing result for `gate`.
    pub fn pass(gate: Gate, details: impl Into<String>) -> Self {
        Self {
            gate,
            passed: true,
            details: details.into(),
        }
    }

    /// A failing result for `gate`.
    pub fn fail(gate: Gate, details: impl Into<String>) -> Self {
        Self {
            gate,
            passed: false,
            details: details.into(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Statement helpers (pure string analysis, no DB connection)
// ─────────────────────────────────────────────────────────────────────────────

/// Split SQL into uppercased, comment-free statements.
fn statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") || trimmed.is_empty() {
            continue;
        }
        current.push_str(trimmed);
        current.push(' ');
        if trimmed.ends_with(';') {
            let stmt = current.trim().trim_end_matches(';').trim().to_string();
            if !stmt.is_empty() {
                out.push(stmt.to_uppercase());
            }
            current.clear();
        }
    }
    let tail = current.trim().trim_end_matches(';').trim().to_string();
    if !tail.is_empty() {
        out.push(tail.to_uppercase());
    }
    out
}

/// Best-effort table names referenced by a statement.
fn referenced_tables(stmt: &str) -> Vec<String> {
    let mut tables = Vec::new();
    let tokens: Vec<&str> = stmt.split_whitespace().collect();
    let keywords = ["TABLE", "INDEX", "INTO", "FROM", "UPDATE", "ON"];
    let mut iter = tokens.iter().peekable();
    while let Some(token) = iter.next() {
        if keywords.contains(token) {
            if let Some(next) = iter.peek() {
                let name = next
                    .trim_matches(|c| c == '"' || c == ';' || c == '(' || c == ',')
                    .to_lowercase();
                if !name.is_empty()
                    && name != "if"
                    && name != "concurrently"
                    && !name.contains('.')
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    // Skip the index name after CREATE INDEX; the table follows ON.
                    if *token != "INDEX" {
                        tables.push(name);
                    }
                }
            }
        }
    }
    tables
}

// ─────────────────────────────────────────────────────────────────────────────
// Gate implementations
// ─────────────────────────────────────────────────────────────────────────────

/// Gate 1: reject statements known to hold long-lived table locks.
fn check_lock_risk(migration: &MigrationFile) -> GateResult {
    for stmt in statements(&migration.up_sql) {
        // Non-concurrent index builds lock writes for the whole build.
        if stmt.contains("CREATE INDEX") && !stmt.contains("CONCURRENTLY") {
            return GateResult::fail(
                Gate::LockRisk,
                format!("blocking index build (missing CONCURRENTLY): {stmt}"),
            );
        }
        if stmt.contains("DROP COLUMN") {
            return GateResult::fail(
                Gate::LockRisk,
                format!("DROP COLUMN takes an ACCESS EXCLUSIVE lock: {stmt}"),
            );
        }
        if stmt.contains("ALTER COLUMN") && stmt.contains("TYPE") {
            return GateResult::fail(
                Gate::LockRisk,
                format!("ALTER COLUMN TYPE rewrites the table: {stmt}"),
            );
        }
        if stmt.contains("ADD CONSTRAINT") && !stmt.contains("NOT VALID") {
            // Validated constraints take a full table lock; NOT VALID + later
            // VALIDATE splits the work.
            return GateResult::fail(
                Gate::LockRisk,
                format!("validated constraint blocks writes; use NOT VALID + VALIDATE: {stmt}"),
            );
        }
        if stmt.contains("CLUSTER") || stmt.contains("VACUUM FULL") {
            return GateResult::fail(
                Gate::LockRisk,
                format!("table-rewriting maintenance blocks access: {stmt}"),
            );
        }
    }
    GateResult::pass(Gate::LockRisk, "no long-lock statements detected")
}

/// Gate 2: reject changes incompatible with the live production schema.
fn check_backward_compat(migration: &MigrationFile, prod: &ProductionSchema) -> GateResult {
    for stmt in statements(&migration.up_sql) {
        for table in referenced_tables(&stmt) {
            let Some(columns) = prod.tables.get(&table.to_lowercase()) else {
                continue;
            };
            // Dropping a column readers still select is a breaking change.
            if stmt.contains("DROP COLUMN") {
                for col in columns {
                    if stmt.contains(&col.name.to_uppercase()) {
                        return GateResult::fail(
                            Gate::BackwardCompatibility,
                            format!(
                                "dropping column {} still present in production table {table}",
                                col.name
                            ),
                        );
                    }
                }
            }
            // Adding NOT NULL without a default breaks existing writers.
            if stmt.contains("ADD COLUMN") && stmt.contains("NOT NULL") && !stmt.contains("DEFAULT")
            {
                return GateResult::fail(
                    Gate::BackwardCompatibility,
                    format!("NOT NULL column without DEFAULT on existing table {table}: {stmt}"),
                );
            }
            // Narrowing a column type breaks readers of wider values.
            if stmt.contains("ALTER COLUMN") && stmt.contains("TYPE") {
                return GateResult::fail(
                    Gate::BackwardCompatibility,
                    format!("column type change on production table {table}: {stmt}"),
                );
            }
        }
        // Partial index changes predicate semantics for existing queries.
        if stmt.contains("CREATE") && stmt.contains("INDEX") && stmt.contains("WHERE") {
            return GateResult::fail(
                Gate::BackwardCompatibility,
                format!("partial index changes query plans for existing readers: {stmt}"),
            );
        }
    }
    GateResult::pass(
        Gate::BackwardCompatibility,
        "compatible with the production schema snapshot",
    )
}

/// Gate 3: require a real rollback script that undoes the forward migration.
fn check_rollback(migration: &MigrationFile) -> GateResult {
    let down = migration.down_sql.trim();
    if down.is_empty() || statements(&migration.down_sql).is_empty() {
        return GateResult::fail(
            Gate::Rollback,
            format!(
                "migration {:04}_{} has no rollback migration",
                migration.version, migration.name
            ),
        );
    }
    let up_tables: Vec<String> = statements(&migration.up_sql)
        .iter()
        .flat_map(|s| referenced_tables(s))
        .collect();
    let down_text = migration.down_sql.to_uppercase();
    if !up_tables
        .iter()
        .all(|t| down_text.contains(&t.to_uppercase()))
    {
        return GateResult::fail(
            Gate::Rollback,
            format!(
                "rollback does not cover forward tables {up_tables:?} for {:04}_{}",
                migration.version, migration.name
            ),
        );
    }
    GateResult::pass(
        Gate::Rollback,
        "rollback migration exists and covers forward tables",
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Report
// ─────────────────────────────────────────────────────────────────────────────

/// Pass/fail artifact for one migration across all gates.
#[derive(Debug, Clone)]
pub struct GateReport {
    /// Migration under review.
    pub version: u32,
    /// Migration name.
    pub name: String,
    /// One result per gate.
    pub results: Vec<GateResult>,
}

impl GateReport {
    /// Whether every gate passed.
    pub fn passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    /// Number of failing gates.
    pub fn failures(&self) -> usize {
        self.results.iter().filter(|r| !r.passed).count()
    }

    /// Render a JUnit-style XML report for existing PR checks.
    pub fn to_junit_xml(&self) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str(&format!(
            "<testsuite name=\"migration-{:04}-{}\" tests=\"{}\" failures=\"{}\">\n",
            self.version,
            xml_escape(&self.name),
            self.results.len(),
            self.failures()
        ));
        for result in &self.results {
            xml.push_str(&format!(
                "  <testcase classname=\"migration-safety\" name=\"{}\">\n",
                result.gate
            ));
            if result.passed {
                xml.push_str("    <system-out>");
                xml.push_str(&xml_escape(&result.details));
                xml.push_str("</system-out>\n");
            } else {
                xml.push_str(&format!(
                    "    <failure message=\"{}\">{}</failure>\n",
                    xml_escape(&result.details),
                    xml_escape(&result.details)
                ));
            }
            xml.push_str("  </testcase>\n");
        }
        xml.push_str("</testsuite>\n");
        xml
    }
}

/// Run all three gates for `migration` against `prod`.
pub fn run_gates(migration: &MigrationFile, prod: &ProductionSchema) -> GateReport {
    GateReport {
        version: migration.version,
        name: migration.name.clone(),
        results: vec![
            check_lock_risk(migration),
            check_backward_compat(migration, prod),
            check_rollback(migration),
        ],
    }
}

/// Minimal XML escaper for report text.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ─────────────────────────────────────────────────────────────────────────────
// Schema snapshot cache
// ─────────────────────────────────────────────────────────────────────────────

/// Process-wide cache of production schema snapshots keyed by snapshot id.
///
/// Loading a snapshot can be expensive (it may be fetched from the snapshot
/// store); caching keeps repeated gate runs well under the 60s CI budget.
fn snapshot_cache() -> &'static Mutex<BTreeMap<String, ProductionSchema>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, ProductionSchema>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Return the cached snapshot for `snapshot_id`, loading it via `load` on a
/// miss. The loader runs at most once per id per process.
pub fn cached_snapshot<F>(snapshot_id: &str, load: F) -> ProductionSchema
where
    F: FnOnce() -> ProductionSchema,
{
    if let Ok(cache) = snapshot_cache().lock() {
        if let Some(snapshot) = cache.get(snapshot_id) {
            return snapshot.clone();
        }
    }
    let snapshot = load();
    if let Ok(mut cache) = snapshot_cache().lock() {
        cache.insert(snapshot_id.to_string(), snapshot.clone());
    }
    snapshot
}

/// Drop all cached snapshots (useful in tests and long-lived processes).
pub fn clear_snapshot_cache() {
    if let Ok(mut cache) = snapshot_cache().lock() {
        cache.clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CI entrypoint
// ─────────────────────────────────────────────────────────────────────────────

/// Outcome of running the gates as a CI step.
#[derive(Debug, Clone)]
pub struct CiOutcome {
    /// JUnit XML report covering every migration.
    pub junit_xml: String,
    /// Number of migrations that failed at least one gate.
    pub failed_migrations: usize,
    /// Total number of migrations analyzed.
    pub total_migrations: usize,
}

impl CiOutcome {
    /// Process exit code: `0` when every migration passed, `1` otherwise.
    pub fn exit_code(&self) -> i32 {
        if self.failed_migrations == 0 {
            0
        } else {
            1
        }
    }
}

/// Run all gates for every migration and render a single JUnit report.
///
/// The returned [`CiOutcome`] carries the XML to write to the CI report path
/// and the exit code the step should return (non-zero on any violation).
pub fn run_ci_gates(
    migrations: &[MigrationFile],
    prod: &ProductionSchema,
) -> CiOutcome {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<testsuites name=\"migration-safety\">\n");
    let mut failed = 0usize;
    for migration in migrations {
        let report = run_gates(migration, prod);
        if !report.passed() {
            failed += 1;
        }
        // Reuse the per-migration suite, stripping its XML declaration.
        let suite = report.to_junit_xml();
        if let Some(idx) = suite.find("<testsuite") {
            xml.push_str(&suite[idx..]);
        }
    }
    xml.push_str("</testsuites>\n");
    CiOutcome {
        junit_xml: xml,
        failed_migrations: failed,
        total_migrations: migrations.len(),
    }
}

/// CI entrypoint: analyze `migrations` against `prod`, write the JUnit XML to
/// `report_path`, and return the process exit code (non-zero on violations).
pub fn ci_main(
    migrations: &[MigrationFile],
    prod: &ProductionSchema,
    report_path: &std::path::Path,
) -> std::io::Result<i32> {
    let outcome = run_ci_gates(migrations, prod);
    if let Some(parent) = report_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(report_path, &outcome.junit_xml)?;
    Ok(outcome.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migration(up: &str, down: &str) -> MigrationFile {
        MigrationFile {
            version: 4,
            name: "test_migration".to_string(),
            up_sql: up.to_string(),
            down_sql: down.to_string(),
        }
    }

    fn prod() -> ProductionSchema {
        ProductionSchema::new().with_table(
            "ledger",
            vec![
                prod_column("seq", "bigint", false),
                prod_column("hash", "text", false),
            ],
        )
    }

    #[test]
    fn safe_concurrent_index_passes_all_gates() {
        let report = run_gates(
            &migration(
                "CREATE INDEX CONCURRENTLY idx_ledger_seq ON ledger (seq);",
                "DROP INDEX idx_ledger_seq;",
            ),
            &prod(),
        );
        assert!(report.passed(), "{report:?}");
        assert_eq!(report.failures(), 0);
    }

    #[test]
    fn anti_pattern_missing_rollback_is_blocked() {
        let report = run_gates(
            &migration("ALTER TABLE ledger ADD COLUMN note TEXT;", ""),
            &prod(),
        );
        assert!(!report.passed());
        assert!(report
            .results
            .iter()
            .any(|r| r.gate == Gate::Rollback && !r.passed));
    }

    #[test]
    fn anti_pattern_breaking_column_drop_is_blocked() {
        let report = run_gates(
            &migration(
                "ALTER TABLE ledger DROP COLUMN hash;",
                "ALTER TABLE ledger ADD COLUMN hash TEXT;",
            ),
            &prod(),
        );
        assert!(!report.passed());
        assert!(report.results.iter().any(|r| !r.passed));
    }

    #[test]
    fn anti_pattern_long_lock_index_is_blocked() {
        let report = run_gates(
            &migration(
                "CREATE INDEX idx_ledger_hash ON ledger (hash);",
                "DROP INDEX idx_ledger_hash;",
            ),
            &prod(),
        );
        assert!(!report.passed());
        assert!(report
            .results
            .iter()
            .any(|r| r.gate == Gate::LockRisk && !r.passed));
    }

    #[test]
    fn anti_pattern_mismatched_type_change_is_blocked() {
        let report = run_gates(
            &migration(
                "ALTER TABLE ledger ALTER COLUMN seq TYPE INTEGER;",
                "ALTER TABLE ledger ALTER COLUMN seq TYPE BIGINT;",
            ),
            &prod(),
        );
        assert!(!report.passed());
    }

    #[test]
    fn anti_pattern_partial_index_is_blocked() {
        let report = run_gates(
            &migration(
                "CREATE INDEX CONCURRENTLY idx_partial ON ledger (seq) WHERE seq > 100;",
                "DROP INDEX idx_partial;",
            ),
            &prod(),
        );
        assert!(!report.passed());
        assert!(report
            .results
            .iter()
            .any(|r| r.gate == Gate::BackwardCompatibility && !r.passed));
    }

    #[test]
    fn rollback_must_cover_forward_tables() {
        let report = run_gates(
            &migration(
                "ALTER TABLE ledger ADD COLUMN note TEXT;",
                "DROP TABLE unrelated;",
            ),
            &ProductionSchema::new(),
        );
        assert!(report
            .results
            .iter()
            .any(|r| r.gate == Gate::Rollback && !r.passed));
    }

    #[test]
    fn junit_report_marks_failures() {
        let report = run_gates(&migration("VACUUM FULL ledger;", ""), &prod());
        assert!(!report.passed());
        let xml = report.to_junit_xml();
        assert!(xml.contains("<testsuite"));
        assert!(xml.contains("<failure"));
        assert!(xml.contains("failures=\""));
    }

    #[test]
    fn junit_report_passes_cleanly_for_safe_migration() {
        let report = run_gates(
            &migration(
                "CREATE INDEX CONCURRENTLY idx_ledger_seq ON ledger (seq);",
                "DROP INDEX idx_ledger_seq;",
            ),
            &prod(),
        );
        let xml = report.to_junit_xml();
        assert!(xml.contains("failures=\"0\""));
        assert!(!xml.contains("<failure"));
    }

    #[test]
    fn gates_run_fast_enough_for_ci() {
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = run_gates(
                &migration(
                    "CREATE INDEX CONCURRENTLY idx_ledger_seq ON ledger (seq);",
                    "DROP INDEX idx_ledger_seq;",
                ),
                &prod(),
            );
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60));
    }

    #[test]
    fn gate_names_render() {
        assert_eq!(Gate::LockRisk.to_string(), "lock-risk");
        assert_eq!(
            Gate::BackwardCompatibility.to_string(),
            "backward-compatibility"
        );
        assert_eq!(Gate::Rollback.to_string(), "rollback");
    }

    #[test]
    fn all_five_anti_patterns_are_blocked() {
        let anti_patterns = [
            // no rollback
            migration("ALTER TABLE ledger ADD COLUMN note TEXT;", ""),
            // breaking column drop
            migration(
                "ALTER TABLE ledger DROP COLUMN hash;",
                "ALTER TABLE ledger ADD COLUMN hash TEXT;",
            ),
            // long lock
            migration(
                "CREATE INDEX idx_ledger_hash ON ledger (hash);",
                "DROP INDEX idx_ledger_hash;",
            ),
            // mismatched type
            migration(
                "ALTER TABLE ledger ALTER COLUMN seq TYPE INTEGER;",
                "ALTER TABLE ledger ALTER COLUMN seq TYPE BIGINT;",
            ),
            // partial index
            migration(
                "CREATE INDEX CONCURRENTLY idx_partial ON ledger (seq) WHERE seq > 100;",
                "DROP INDEX idx_partial;",
            ),
        ];
        for m in &anti_patterns {
            let report = run_gates(m, &prod());
            assert!(!report.passed(), "anti-pattern not blocked: {m:?}");
        }
    }

    #[test]
    fn ci_outcome_exit_code_reflects_violations() {
        let clean = run_ci_gates(
            &[migration(
                "CREATE INDEX CONCURRENTLY idx_ledger_seq ON ledger (seq);",
                "DROP INDEX idx_ledger_seq;",
            )],
            &prod(),
        );
        assert_eq!(clean.exit_code(), 0);
        assert_eq!(clean.failed_migrations, 0);
        assert!(clean.junit_xml.contains("<testsuites"));

        let dirty = run_ci_gates(&[migration("VACUUM FULL ledger;", "")], &prod());
        assert_eq!(dirty.exit_code(), 1);
        assert_eq!(dirty.failed_migrations, 1);
        assert!(dirty.junit_xml.contains("<failure"));
    }

    #[test]
    fn ci_main_writes_report_and_returns_code() {
        let dir = std::env::temp_dir().join(format!(
            "migration-safety-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("report.xml");
        let code = ci_main(&[migration("VACUUM FULL ledger;", "")], &prod(), &path).unwrap();
        assert_eq!(code, 1);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("<testsuites"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_cache_loads_once() {
        clear_snapshot_cache();
        let calls = std::cell::Cell::new(0);
        let first = cached_snapshot("prod-2026-01", || {
            calls.set(calls.get() + 1);
            prod()
        });
        let second = cached_snapshot("prod-2026-01", || {
            calls.set(calls.get() + 1);
            prod()
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(first, second);
        clear_snapshot_cache();
    }
}
