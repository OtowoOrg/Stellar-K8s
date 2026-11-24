//! Integration test for the DB compaction daemon's database pieces.
//!
//! These tests require a reachable PostgreSQL instance with a dedicated
//! scratch database. Point `TEST_DATABASE_URL` at it (e.g.
//! `postgres://user:pass@localhost:5432/compaction_test`), then run:
//!
//! ```text
//! TEST_DATABASE_URL=postgres://... cargo test --test maintenance_compaction_test
//! ```
//!
//! Without `TEST_DATABASE_URL` the tests skip cleanly so CI does not need a
//! live database. All tables created here are dropped at the end.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use stellar_k8s::controller::maintenance::{
    evaluate_fragmentation, total_relation_size, DatabaseIntegrityVerifier, LedgerPruner,
};

const TABLE: &str = "public.compaction_test_items";
const LEDGER_TABLE: &str = "public.history_ledgers";

async fn connect() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    match PgPoolOptions::new().max_connections(4).connect(&url).await {
        Ok(pool) => Some(pool),
        Err(e) => {
            eprintln!("warning: TEST_DATABASE_URL set but connection failed: {e}");
            None
        }
    }
}

/// Create a scratch table, seed it with rows, and leave dead tuples behind by
/// deleting every other row. Returns the number of live rows remaining.
async fn seed_fragmented_table(pool: &PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query(&format!("DROP TABLE IF EXISTS {TABLE}"))
        .execute(pool)
        .await?;
    sqlx::query(&format!(
        "CREATE TABLE {TABLE} (id bigint PRIMARY KEY, payload text NOT NULL)"
    ))
    .execute(pool)
    .await?;

    let insert_count = 200_000u32;
    for start in (0..insert_count).step_by(10_000) {
        let mut query = String::from("INSERT INTO compaction_test_items (id, payload) VALUES ");
        let batch: Vec<String> = (start..(start + 10_000).min(insert_count))
            .map(|i| {
                format!(
                    "({i}, 'payload-{i}-{}-{}', repeat('x', 128))",
                    i % 7,
                    i % 13
                )
            })
            .collect();
        query.push_str(&batch.join(", "));
        sqlx::query(&query).execute(pool).await?;
    }

    // Delete every other row → ~50% dead tuples, well above the 30% threshold.
    let result = sqlx::query("DELETE FROM compaction_test_items WHERE id % 2 = 0")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Seed a Horizon-shaped `history_ledgers` table spanning ~120 days.
async fn seed_history_ledgers(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(&format!("DROP TABLE IF EXISTS {LEDGER_TABLE}"))
        .execute(pool)
        .await?;
    sqlx::query(&format!(
        "CREATE TABLE {LEDGER_TABLE} (sequence bigint PRIMARY KEY, closed_at timestamptz NOT NULL)"
    ))
    .execute(pool)
    .await?;

    let ledgers = 5000u32;
    for start in (0..ledgers).step_by(1_000) {
        let mut query = String::from("INSERT INTO history_ledgers (sequence, closed_at) VALUES ");
        let batch: Vec<String> = (start..(start + 1_000).min(ledgers))
            .map(|i| {
                // Newest sequence (i) closed most recently; spread over 120 days.
                let age_days = (ledgers - 1 - i) as f64 / 5000.0 * 120.0;
                format!("({i}, NOW() - INTERVAL '{age_days} days')")
            })
            .collect();
        query.push_str(&batch.join(", "));
        sqlx::query(&query).execute(pool).await?;
    }
    Ok(())
}

/// Verify VACUUM FULL + REINDEX compacts a fragmented table without changing
/// the data, and that post-compaction checksums match.
#[tokio::test]
async fn test_compact_and_verify_integrity() {
    let Some(pool) = connect().await else {
        eprintln!("SKIPPED: TEST_DATABASE_URL not set");
        return;
    };

    let live = seed_fragmented_table(&pool).await.unwrap();
    assert_eq!(live, 100_000, "half the rows should be deleted");
    sqlx::query("ANALYZE compaction_test_items")
        .execute(&pool)
        .await
        .unwrap();

    // Fragmentation is detected.
    let metrics = evaluate_fragmentation(&pool, 30, 0).await.unwrap();
    let item = metrics
        .iter()
        .find(|m| m.qualified_name == TABLE)
        .expect("table must appear in fragmentation report");
    assert!(
        item.fragmented,
        "table with ~50% dead tuples should be flagged as fragmented"
    );

    // Pre-compaction checksum + size.
    let verifier = DatabaseIntegrityVerifier::new(pool.clone());
    let before = verifier
        .compute_table_checksum(TABLE)
        .await
        .expect("pre-compaction checksum");
    let size_before = total_relation_size(&pool, &[TABLE.to_string()])
        .await
        .unwrap();

    // Run the actual compaction.
    sqlx::query(format!("VACUUM (FULL, ANALYZE) {TABLE}").as_str())
        .execute(&pool)
        .await
        .expect("VACUUM FULL");

    let size_after = total_relation_size(&pool, &[TABLE.to_string()])
        .await
        .unwrap();

    // Post-compaction integrity: data is intact and the file footprint shrank.
    let after = verifier
        .compute_table_checksum(TABLE)
        .await
        .expect("post-compaction checksum");
    assert_eq!(
        before, after,
        "VACUUM FULL must preserve the data (checksums must match)"
    );
    assert!(
        size_after <= size_before,
        "compaction should not grow the table (before={size_before}, after={size_after})"
    );
    eprintln!("integrity OK: checksums match, size {size_before} → {size_after} bytes");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM compaction_test_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 100_000, "row count preserved after compaction");

    let _ = sqlx::query(&format!("DROP TABLE IF EXISTS {TABLE}"))
        .execute(&pool)
        .await;
}

/// Verify ledger pruning deletes rows outside the retention window while
/// respecting the safety buffer of the newest ledgers.
#[tokio::test]
async fn test_ledger_pruning_respects_retention_and_safety() {
    let Some(pool) = connect().await else {
        eprintln!("SKIPPED: TEST_DATABASE_URL not set");
        return;
    };

    seed_history_ledgers(&pool).await.unwrap();

    let pruner = LedgerPruner::new(pool.clone(), 10); // keep 10 days
    let report = pruner.prune_ledgers().await.expect("prune");

    // We inserted 5000 ledgers over 120 days. After pruning, the newest
    // MIN_KEEP_LEDGERS (1000) must survive; older ones are gone.
    eprintln!("prune report: {report:?}");
    assert!(report.ledgers_deleted > 0, "must delete old ledgers");

    let min_seq: i64 = sqlx::query_scalar("SELECT MIN(sequence) FROM history_ledgers")
        .fetch_one(&pool)
        .await
        .unwrap();
    let max_seq: i64 = sqlx::query_scalar("SELECT MAX(sequence) FROM history_ledgers")
        .fetch_one(&pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_ledgers")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(max_seq, 4_999, "the newest ledger must always be retained");
    assert_eq!(count, 1_000, "safety buffer keeps the newest 1000 ledgers");
    assert_eq!(min_seq, 4_000, "boundary is the first sequence kept");

    let _ = sqlx::query(&format!("DROP TABLE IF EXISTS {LEDGER_TABLE}"))
        .execute(&pool)
        .await;
}
