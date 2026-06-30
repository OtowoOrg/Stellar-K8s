//! Automated compliance reporting for regulatory requirements.
//!
//! Provides continuous compliance monitoring, validation pipelines for
//! SOC2, GDPR, and PCI-DSS, automated report generation, and evidence collection.

pub mod audit_trail;
pub mod dashboard;
pub mod evidence;
pub mod export;
pub mod frameworks;
pub mod monitor;
pub mod opa;
pub mod policy_engine;
pub mod policy_test;
pub mod policy_version;
pub mod report;
pub mod soc2_iso;

pub use evidence::{EvidenceCollector, EvidenceItem};
pub use export::{export_csv, export_json, export_pdf, ComplianceExportFormat};
pub use frameworks::{ComplianceFramework, ComplianceRule, RuleResult, ValidationPipeline};
pub use monitor::{ComplianceMonitor, ComplianceStatus, DriftFinding};
pub use report::{ComplianceReport, ReportGenerator};
