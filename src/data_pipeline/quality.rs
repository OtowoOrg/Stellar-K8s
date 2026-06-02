//! Data quality validation and cleansing rules for the Stellar data pipeline

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

use super::etl::EtlRecord;

/// Severity level for quality failures
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// A single quality violation found during validation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QualityViolation {
    pub rule_name: String,
    pub severity: Severity,
    pub field: String,
    pub message: String,
    pub record_sequence: u64,
}

/// Summary report for a batch of validated records
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QualityReport {
    pub total_records: usize,
    pub passed: usize,
    pub failed: usize,
    pub violations: Vec<QualityViolation>,
    pub violation_counts_by_rule: HashMap<String, usize>,
    pub pass_rate_pct: f64,
}

impl QualityReport {
    pub fn has_critical(&self) -> bool {
        self.violations.iter().any(|v| v.severity == Severity::Critical)
    }
}

/// A validation rule applied to ETL records
pub struct ValidationRule {
    pub name: String,
    pub severity: Severity,
    check: Box<dyn Fn(&EtlRecord) -> Option<(String, String)> + Send + Sync>,
}

impl ValidationRule {
    pub fn new(
        name: impl Into<String>,
        severity: Severity,
        check: impl Fn(&EtlRecord) -> Option<(String, String)> + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), severity, check: Box::new(check) }
    }

    pub fn validate(&self, record: &EtlRecord) -> Option<QualityViolation> {
        (self.check)(record).map(|(field, message)| QualityViolation {
            rule_name: self.name.clone(),
            severity: self.severity.clone(),
            field,
            message,
            record_sequence: record.sequence,
        })
    }
}

/// Engine that applies all validation rules and produces quality reports
pub struct DataQualityEngine {
    rules: Vec<ValidationRule>,
}

impl DataQualityEngine {
    /// Create engine with the default Stellar ledger quality rules
    pub fn with_default_rules() -> Self {
        let mut engine = Self { rules: Vec::new() };
        engine.add_default_rules();
        engine
    }

    pub fn add_rule(&mut self, rule: ValidationRule) {
        self.rules.push(rule);
    }

    fn add_default_rules(&mut self) {
        // Rule 1: ledger sequence must be positive
        self.add_rule(ValidationRule::new(
            "positive_sequence",
            Severity::Critical,
            |r| {
                if r.sequence == 0 {
                    Some(("sequence".into(), "Ledger sequence must be > 0".into()))
                } else {
                    None
                }
            },
        ));

        // Rule 2: hash must be non-empty
        self.add_rule(ValidationRule::new("non_empty_hash", Severity::Critical, |r| {
            if r.hash.is_empty() {
                Some(("hash".into(), "Ledger hash must not be empty".into()))
            } else {
                None
            }
        }));

        // Rule 3: base fee must be at least 100 stroops (0.00001 XLM)
        self.add_rule(ValidationRule::new("min_base_fee", Severity::Warning, |r| {
            if r.base_fee_xlm < 0.000_001 {
                Some(("base_fee_xlm".into(), format!("Base fee too low: {}", r.base_fee_xlm)))
            } else {
                None
            }
        }));

        // Rule 4: success rate must be in [0,1]
        self.add_rule(ValidationRule::new("valid_success_rate", Severity::Error, |r| {
            if !(0.0..=1.0).contains(&r.tx_success_rate) {
                Some((
                    "tx_success_rate".into(),
                    format!("Success rate out of range: {}", r.tx_success_rate),
                ))
            } else {
                None
            }
        }));

        // Rule 5: date partition must match YYYY-MM-DD
        self.add_rule(ValidationRule::new("date_partition_format", Severity::Error, |r| {
            let valid = r.date_partition.len() == 10
                && r.date_partition.chars().nth(4) == Some('-')
                && r.date_partition.chars().nth(7) == Some('-');
            if !valid {
                Some((
                    "date_partition".into(),
                    format!("Invalid date partition format: {}", r.date_partition),
                ))
            } else {
                None
            }
        }));

        // Rule 6: avg ops per tx should not exceed 100 (sanity check)
        self.add_rule(ValidationRule::new("ops_per_tx_sanity", Severity::Warning, |r| {
            if r.avg_ops_per_tx > 100.0 {
                Some((
                    "avg_ops_per_tx".into(),
                    format!("Unusually high ops/tx: {:.1}", r.avg_ops_per_tx),
                ))
            } else {
                None
            }
        }));
    }

    /// Validate a single record
    pub fn validate(&self, record: &EtlRecord) -> Vec<QualityViolation> {
        self.rules.iter().filter_map(|r| r.validate(record)).collect()
    }

    /// Validate a batch and produce a summary report
    pub fn validate_batch(&self, records: &[EtlRecord]) -> QualityReport {
        let mut report = QualityReport {
            total_records: records.len(),
            ..Default::default()
        };

        for record in records {
            let violations = self.validate(record);
            if violations.is_empty() {
                report.passed += 1;
            } else {
                report.failed += 1;
                for v in &violations {
                    *report.violation_counts_by_rule.entry(v.rule_name.clone()).or_insert(0) += 1;
                }
                report.violations.extend(violations);
            }
        }

        report.pass_rate_pct = if report.total_records > 0 {
            report.passed as f64 / report.total_records as f64 * 100.0
        } else {
            100.0
        };

        if report.has_critical() {
            warn!(
                critical_violations = report.violations.iter().filter(|v| v.severity == Severity::Critical).count(),
                "Critical data quality violations detected"
            );
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_pipeline::etl::LedgerSizeCategory;
    use std::collections::HashMap;

    fn good_record(seq: u64) -> EtlRecord {
        EtlRecord {
            sequence: seq,
            hash: format!("hash_{seq:016x}"),
            base_fee_xlm: 0.00001,
            base_reserve_xlm: 0.5,
            timestamp_epoch_ms: 1_700_000_000_000,
            date_partition: "2024-01-15".into(),
            hour_partition: 12,
            tx_success_rate: 0.98,
            avg_ops_per_tx: 2.5,
            ledger_size_category: LedgerSizeCategory::Medium,
            pipeline_version: "1.0.0".into(),
            enriched_at: chrono::Utc::now(),
            tags: HashMap::new(),
        }
    }

    #[test]
    fn test_good_record_passes() {
        let engine = DataQualityEngine::with_default_rules();
        let violations = engine.validate(&good_record(100));
        assert!(violations.is_empty());
    }

    #[test]
    fn test_zero_sequence_is_critical() {
        let engine = DataQualityEngine::with_default_rules();
        let mut r = good_record(0);
        r.sequence = 0;
        let violations = engine.validate(&r);
        assert!(violations.iter().any(|v| v.severity == Severity::Critical));
    }

    #[test]
    fn test_invalid_success_rate_is_error() {
        let engine = DataQualityEngine::with_default_rules();
        let mut r = good_record(1);
        r.tx_success_rate = 1.5;
        let violations = engine.validate(&r);
        assert!(violations.iter().any(|v| v.rule_name == "valid_success_rate"));
    }

    #[test]
    fn test_batch_pass_rate() {
        let engine = DataQualityEngine::with_default_rules();
        let records: Vec<EtlRecord> = (1..=10).map(good_record).collect();
        let report = engine.validate_batch(&records);
        assert_eq!(report.passed, 10);
        assert!((report.pass_rate_pct - 100.0).abs() < 0.01);
    }
}
