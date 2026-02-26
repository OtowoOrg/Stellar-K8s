//! Database bloat detection for Postgres
//!
//! Queries Postgres statistics tables to estimate table and index bloat.

use crate::error::Result;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

pub struct BloatDetector {
    pool: PgPool,
}

impl BloatDetector {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Estimate bloat percentage for a specific table
    pub async fn estimate_table_bloat(&self, table_name: &str) -> Result<f64> {
        // Standard bloat estimation query for Postgres
        let query = r#"
            SELECT
              current_database(), schemaname, relname, 
              ROUND(CASE WHEN otta=0 THEN 0 ELSE sml.relpages/otta::numeric END,1) AS tbloat
            FROM (
              SELECT
                schemaname, tablename, relpages, est_pgfree,
                CEIL((reltuples*slot_size*(1-fillfactor/100.0))/bs) AS otta
              FROM (
                SELECT
                  schemaname, tablename, reltuples, relpages, fillfactor, bs,
                  CASE WHEN reltuples=0 THEN 0 ELSE (relpages*bs)/reltuples END AS slot_size,
                  (SELECT (regexp_matches(current_setting('block_size'), E'\\d+'))[1]::int) AS est_pgfree
                FROM pg_catalog.pg_tables
                JOIN pg_catalog.pg_class ON (relname = tablename)
                JOIN pg_catalog.pg_namespace ON (pg_namespace.oid = relnamespace)
                WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
              ) AS storage_info
            ) AS sml
            WHERE relname = $1
        "#;

        let row: PgRow = sqlx::query(query)
            .bind(table_name)
            .fetch_one(&self.pool)
            .await?;

        let bloat: f64 = row.try_get("tbloat")?;
        Ok(bloat)
    }

    /// List bloated tables exceeding a threshold
    pub async fn get_bloated_tables(&self, threshold_percent: u32) -> Result<Vec<String>> {
        // Simplistic implementation for now
        let query = "SELECT relname FROM pg_class WHERE relkind = 'r'";
        let rows: Vec<PgRow> = sqlx::query(query).fetch_all(&self.pool).await?;

        let mut bloated = Vec::new();
        for row in rows {
            let table: String = Row::get::<String, usize>(&row, 0);
            if self.estimate_table_bloat(&table).await? > threshold_percent as f64 {
                bloated.push(table);
            }
        }
        Ok(bloated)
    }
}
