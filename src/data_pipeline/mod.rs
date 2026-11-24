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

pub mod config;
pub mod dead_letter;
pub mod etl;
pub mod ingestion;
pub mod lineage;
pub mod metrics;
pub mod monitoring;
pub mod partitioning;
pub mod pipeline;
pub mod quality;
pub mod sinks;
pub mod warehouse;

pub use config::PipelineConfig;
pub use dead_letter::{DeadLetterQueue, FailedRecord};
pub use etl::{EtlRecord, TransformError};
pub use ingestion::{LedgerIngestion, LedgerRecord, StreamConfig};
pub use lineage::LineageTracker;
pub use metrics::PipelineMetrics;
pub use monitoring::PipelineMonitor;
pub use partitioning::{PartitionKey, PartitionStrategy};
pub use pipeline::{DataPipeline, PipelineHandle};
pub use quality::{DataQualityEngine, QualityReport, ValidationRule};
pub use sinks::SinkError;
pub use warehouse::{WarehouseAdapter, WarehouseConfig, WarehouseProvider};
