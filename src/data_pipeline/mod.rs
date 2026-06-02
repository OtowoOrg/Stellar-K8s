//! Advanced Data Pipeline with Stream Processing and ETL for Stellar Ledger Data
//!
//! Provides real-time ledger ingestion, ETL transformations, data quality validation,
//! warehouse integration, pipeline monitoring, and data lineage tracking.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │  Stellar Data Pipeline                                        │
//! ├──────────────────────────────────────────────────────────────┤
//! │  Stellar Core  →  Ingestion  →  ETL  →  Validation          │
//! │                                    ↓                         │
//! │                    Partitioning / Indexing                   │
//! │                                    ↓                         │
//! │           Warehouse Adapter (Snowflake / BigQuery)            │
//! │                                    ↓                         │
//! │           Pipeline Monitor + Dead-Letter Queue               │
//! └──────────────────────────────────────────────────────────────┘
//! ```

pub mod dead_letter;
pub mod etl;
pub mod ingestion;
pub mod lineage;
pub mod monitoring;
pub mod partitioning;
pub mod quality;
pub mod warehouse;

pub use dead_letter::{DeadLetterQueue, FailedRecord};
pub use etl::{EtlPipeline, EtlRecord, TransformResult};
pub use ingestion::{LedgerIngestion, LedgerRecord, StreamConfig};
pub use lineage::{DataLineage, LineageEvent};
pub use monitoring::{PipelineMetrics, PipelineMonitor};
pub use partitioning::{PartitionKey, PartitionStrategy};
pub use quality::{DataQualityEngine, QualityReport, ValidationRule};
pub use warehouse::{WarehouseAdapter, WarehouseConfig, WarehouseProvider};
