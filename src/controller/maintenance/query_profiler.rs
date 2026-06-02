//! Query profiling and index recommendations for PostgreSQL.
//!
//! This module provides a lightweight analysis layer that inspects
//! PostgreSQL slow query statistics and suggests indexes for common
//! Horizon/Soroban workloads.

use crate::error::Result;
use regex::Regex;
use sqlx::{PgPool, PgRow, Row};

/// A slow query candidate from `pg_stat_statements`.
pub struct SlowQuery {
    pub query: String,
    pub calls: i64,
    pub avg_ms: f64,
    pub total_ms: f64,
}

/// A suggested index for a slow query.
pub struct IndexSuggestion {
    pub table: String,
    pub columns: Vec<String>,
    pub index_name: String,
    pub query: String,
}

pub struct QueryProfiler {
    pool: PgPool,
}

impl QueryProfiler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Collect slow queries from PostgreSQL if `pg_stat_statements` is enabled.
    pub async fn collect_slow_queries(&self, threshold_ms: u32) -> Result<Vec<SlowQuery>> {
        let query = r#"
            SELECT query, calls, total_time, rows
            FROM pg_stat_statements
            WHERE calls > 5
              AND total_time / calls > $1
            ORDER BY total_time DESC
            LIMIT 10
        "#;

        let rows: Vec<PgRow> = match sqlx::query(query)
            .bind(threshold_ms as f64)
            .fetch_all(&self.pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                if let sqlx::Error::Database(db_err) = &e {
                    let msg = db_err.message().to_lowercase();
                    if msg.contains("pg_stat_statements") || msg.contains("relation \"pg_stat_statements\"") {
                        return Ok(Vec::new());
                    }
                }
                return Err(e.into());
            }
        };

        let mut slow_queries = Vec::new();
        for row in rows {
            let query_text: String = row.try_get("query")?;
            let calls: i64 = row.try_get("calls")?;
            let total_ms: f64 = row.try_get("total_time")?;
            let avg_ms = if calls > 0 { total_ms / calls as f64 } else { 0.0 };
            slow_queries.push(SlowQuery {
                query: query_text,
                calls,
                avg_ms,
                total_ms,
            });
        }

        Ok(slow_queries)
    }

    /// Analyze slow queries and suggest indexes for equality filters.
    pub fn recommend_indexes(&self, slow_queries: &[SlowQuery]) -> Vec<IndexSuggestion> {
        let table_re = Regex::new(r"(?i)\bFROM\s+([a-zA-Z_][\w\.]*)(?:\s|$)").unwrap();
        let equality_re = Regex::new(r"(?i)(?:WHERE|AND|OR)\s+([a-zA-Z_][\w\.]*)\s*=\s*[$]?[0-9]+(?:\b|\s)").unwrap();

        let mut suggestions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for query in slow_queries {
            if let Some(table_match) = table_re.captures(&query.query) {
                if let Some(table_name) = table_match.get(1) {
                    let table = table_name.as_str().to_string();
                    let mut columns = Vec::new();
                    for cap in equality_re.captures_iter(&query.query) {
                        if let Some(col_match) = cap.get(1) {
                            let column = col_match.as_str();
                            let column = column
                                .split('.')
                                .last()
                                .unwrap_or(column)
                                .to_string();
                            if !columns.contains(&column) {
                                columns.push(column);
                            }
                        }
                    }
                    if !columns.is_empty() {
                        let key = format!("{}:{}", table, columns.join(","));
                        if seen.insert(key.clone()) {
                            let index_name = format!(
                                "idx_{}_{}",
                                table.replace('.', "_"),
                                columns.join("_")
                            );
                            suggestions.push(IndexSuggestion {
                                table,
                                columns,
                                index_name,
                                query: query.query.clone(),
                            });
                        }
                    }
                }
            }
        }

        suggestions
    }

    /// Ensure suggested indexes exist by creating them concurrently if necessary.
    pub async fn ensure_indexes(&self, suggestions: &[IndexSuggestion]) -> Result<()> {
        for suggestion in suggestions {
            self.create_index(suggestion).await?;
        }
        Ok(())
    }

    /// Create a PostgreSQL index concurrently.
    pub async fn create_index(&self, suggestion: &IndexSuggestion) -> Result<()> {
        let columns = suggestion.columns.join(", ");
        let sql = format!(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS {} ON {} ({})",
            suggestion.index_name, suggestion.table, columns
        );
        sqlx::query(&sql).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommend_indexes_parses_where_clauses() {
        let profiler = QueryProfiler { pool: PgPool::connect_lazy("postgres://localhost/test") };
        let query = SlowQuery {
            query: "SELECT * FROM payments WHERE payment_id = $1 AND ledger_seq = $2".into(),
            calls: 10,
            avg_ms: 250.0,
            total_ms: 2500.0,
        };

        let suggestions = profiler.recommend_indexes(&[query]);
        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        assert_eq!(suggestion.table, "payments");
        assert_eq!(suggestion.columns, vec!["payment_id".to_string(), "ledger_seq".to_string()]);
        assert!(suggestion.index_name.starts_with("idx_payments_payment_id_ledger_seq"));
    }
}
