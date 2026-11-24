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
        self.violations
            .iter()
            .any(|v| v.severity == Severity::Critical)
    }
}

/// (field, message) emitted by a [`ValidationRule`] check when validation fails
type CheckFn = dyn Fn(&EtlRecord) -> Option<(String, String)> + Send + Sync;

/// A validation rule applied to ETL records
pub struct ValidationRule {
    pub name: String,
    pub severity: Severity,
    check: Box<CheckFn>,
}

impl ValidationRule {
    pub fn new(
        name: impl Into<String>,
        severity: Severity,
        check: impl Fn(&EtlRecord) -> Option<(String, String)> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            severity,
            check: Box::new(check),
        }
    }

    pub fn validate(&self, record: &EtlRecord) -> Option<QualityViolation> {
        (self.check)(record).map(|(field, message)| QualityViolation {
            rule_name: self.name.clone(),
            severity: self.severity.clone(),
            field,
            message,
            record_sequence: record.ledger_seq.unwrap_or(0),
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
                let sequence = r.ledger_seq.unwrap_or(0);
                if sequence == 0 {
                    Some(("sequence".into(), "Ledger sequence must be > 0".into()))
                } else {
                    None
                }
            },
        ));

        // Rule 2: hash must be non-empty
        self.add_rule(ValidationRule::new(
            "non_empty_hash",
            Severity::Critical,
            |r| {
                let hash = r
                    .payload
                    .get("hash")
                    .or_else(|| r.payload.get("ledger_hash"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if hash.is_empty() {
                    Some(("hash".into(), "Ledger hash must not be empty".into()))
                } else {
                    None
                }
            },
        ));

        // Rule 3: base fee must be at least 100 stroops (0.00001 XLM)
        self.add_rule(ValidationRule::new(
            "min_base_fee",
            Severity::Warning,
            |r| {
                let base_fee = r
                    .payload
                    .get("base_fee_xlm")
                    .or_else(|| r.payload.get("base_fee"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if base_fee < 0.000_001 {
                    Some((
                        "base_fee_xlm".into(),
                        format!("Base fee too low: {}", base_fee),
                    ))
                } else {
                    None
                }
            },
        ));

        // Rule 4: success rate must be in [0,1]
        self.add_rule(ValidationRule::new(
            "valid_success_rate",
            Severity::Error,
            |r| {
                let success_rate = r
                    .payload
                    .get("tx_success_rate")
                    .or_else(|| r.payload.get("success_rate"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if !(0.0..=1.0).contains(&success_rate) {
                    Some((
                        "tx_success_rate".into(),
                        format!("Success rate out of range: {}", success_rate),
                    ))
                } else {
                    None
                }
            },
        ));

        // Rule 5: date partition must match YYYY-MM-DD
        self.add_rule(ValidationRule::new(
            "date_partition_format",
            Severity::Error,
            |r| {
                let date_partition = r
                    .metadata
                    .get("date_partition")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| r.pipeline_ts.split('T').next().unwrap_or("").to_string());
                let valid = date_partition.len() == 10
                    && date_partition.chars().nth(4) == Some('-')
                    && date_partition.chars().nth(7) == Some('-');
                if !valid {
                    Some((
                        "date_partition".into(),
                        format!("Invalid date partition format: {}", date_partition),
                    ))
                } else {
                    None
                }
            },
        ));

        // Rule 6: avg ops per tx should not exceed 100 (sanity check)
        self.add_rule(ValidationRule::new(
            "ops_per_tx_sanity",
            Severity::Warning,
            |r| {
                let avg_ops = r
                    .payload
                    .get("avg_ops_per_tx")
                    .or_else(|| r.payload.get("operations_per_transaction"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if avg_ops > 100.0 {
                    Some((
                        "avg_ops_per_tx".into(),
                        format!("Unusually high ops/tx: {:.1}", avg_ops),
                    ))
                } else {
                    None
                }
            },
        ));
    }

    /// Validate a single record
    pub fn validate(&self, record: &EtlRecord) -> Vec<QualityViolation> {
        self.rules
            .iter()
            .filter_map(|r| r.validate(record))
            .collect()
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
                    *report
                        .violation_counts_by_rule
                        .entry(v.rule_name.clone())
                        .or_insert(0) += 1;
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
                critical_violations = report
                    .violations
                    .iter()
                    .filter(|v| v.severity == Severity::Critical)
                    .count(),
                "Critical data quality violations detected"
            );
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn good_record(seq: u64) -> EtlRecord {
        let mut metadata = HashMap::new();
        metadata.insert("date_partition".into(), "2024-01-15".into());
        EtlRecord {
            id: format!("test-{seq}"),
            source_topic: "ledger".into(),
            partition: 0,
            offset: seq as i64,
            payload: serde_json::json!({
                "hash": format!("hash_{seq:016x}"),
                "base_fee_xlm": 0.00001,
                "base_reserve_xlm": 0.5,
                "tx_success_rate": 0.98,
                "avg_ops_per_tx": 2.5,
                "ledger_size_category": "medium",
            }),
            metadata,
            pipeline_ts: "2024-01-15T12:00:00Z".into(),
            ledger_seq: Some(seq),
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
        r.ledger_seq = Some(0);
        let violations = engine.validate(&r);
        assert!(violations.iter().any(|v| v.severity == Severity::Critical));
    }

    #[test]
    fn test_invalid_success_rate_is_error() {
        let engine = DataQualityEngine::with_default_rules();
        let mut r = good_record(1);
        r.payload["tx_success_rate"] = serde_json::json!(1.5);
        let violations = engine.validate(&r);
        assert!(violations
            .iter()
            .any(|v| v.rule_name == "valid_success_rate"));
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
