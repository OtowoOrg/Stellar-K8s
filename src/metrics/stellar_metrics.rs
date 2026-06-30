//! Enhanced Stellar-specific Prometheus metrics exporter
//!
//! This module provides comprehensive metrics for Stellar nodes including:
//! - Ledger close time metrics
//! - Transaction throughput metrics
//! - Peer connection quality metrics
//! - History archive health metrics
//! - Database size and growth metrics

use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, gauge::Gauge, histogram::Histogram, family::Family},
    registry::Registry,
};
use std::sync::Arc;

/// Labels for Stellar node metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct NodeLabels {
    /// Node name
    pub node_name: String,
    /// Node type (validator, horizon, soroban-rpc)
    pub node_type: String,
    /// Kubernetes namespace
    pub namespace: String,
    /// Stellar network (mainnet, testnet, futurenet)
    pub network: String,
}

/// Labels for peer connection metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct PeerLabels {
    /// Node name
    pub node_name: String,
    /// Peer ID
    pub peer_id: String,
    /// Connection state (authenticated, pending, failed)
    pub state: String,
}

/// Labels for history archive metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ArchiveLabels {
    /// Node name
    pub node_name: String,
    /// Archive name
    pub archive_name: String,
    /// Archive URL
    pub archive_url: String,
}

/// Comprehensive Stellar metrics registry
pub struct StellarMetrics {
    // Ledger metrics
    pub ledger_close_time_seconds: Family<NodeLabels, Histogram>,
    pub ledger_close_time_p50: Family<NodeLabels, Gauge>,
    pub ledger_close_time_p95: Family<NodeLabels, Gauge>,
    pub ledger_close_time_p99: Family<NodeLabels, Gauge>,
    pub ledger_operations_total: Family<NodeLabels, Counter>,
    pub ledger_transactions_total: Family<NodeLabels, Counter>,
    pub ledger_failed_transactions_total: Family<NodeLabels, Counter>,
    
    // Transaction throughput metrics
    pub transaction_throughput_tps: Family<NodeLabels, Gauge>,
    pub transaction_throughput_ops: Family<NodeLabels, Gauge>,
    pub transaction_apply_time_seconds: Family<NodeLabels, Histogram>,
    pub transaction_queue_size: Family<NodeLabels, Gauge>,
    pub transaction_success_rate: Family<NodeLabels, Gauge>,
    
    // Peer connection quality metrics
    pub peer_connection_count: Family<PeerLabels, Gauge>,
    pub peer_message_latency_ms: Family<PeerLabels, Histogram>,
    pub peer_messages_sent_total: Family<PeerLabels, Counter>,
    pub peer_messages_received_total: Family<PeerLabels, Counter>,
    pub peer_connection_errors_total: Family<PeerLabels, Counter>,
    pub peer_bandwidth_bytes_sent: Family<PeerLabels, Counter>,
    pub peer_bandwidth_bytes_received: Family<PeerLabels, Counter>,
    pub peer_connection_uptime_seconds: Family<PeerLabels, Gauge>,
    
    // History archive health metrics
    pub archive_health_status: Family<ArchiveLabels, Gauge>,
    pub archive_last_check_timestamp: Family<ArchiveLabels, Gauge>,
    pub archive_check_duration_seconds: Family<ArchiveLabels, Histogram>,
    pub archive_missing_files_total: Family<ArchiveLabels, Counter>,
    pub archive_download_errors_total: Family<ArchiveLabels, Counter>,
    pub archive_upload_errors_total: Family<ArchiveLabels, Counter>,
    pub archive_size_bytes: Family<ArchiveLabels, Gauge>,
    
    // Database metrics
    pub database_size_bytes: Family<NodeLabels, Gauge>,
    pub database_growth_rate_bytes_per_hour: Family<NodeLabels, Gauge>,
    pub database_query_duration_seconds: Family<NodeLabels, Histogram>,
    pub database_connection_pool_active: Family<NodeLabels, Gauge>,
    pub database_connection_pool_idle: Family<NodeLabels, Gauge>,
    pub database_transaction_count: Family<NodeLabels, Counter>,
    pub database_slow_queries_total: Family<NodeLabels, Counter>,
    
    // SCP (Stellar Consensus Protocol) metrics
    pub scp_nomination_time_seconds: Family<NodeLabels, Histogram>,
    pub scp_ballot_prepare_time_seconds: Family<NodeLabels, Histogram>,
    pub scp_ballot_commit_time_seconds: Family<NodeLabels, Histogram>,
    pub scp_externalize_time_seconds: Family<NodeLabels, Histogram>,
    pub scp_quorum_intersection_failures: Family<NodeLabels, Counter>,
    
    // Soroban-specific metrics
    pub soroban_contract_invocations_total: Family<NodeLabels, Counter>,
    pub soroban_contract_execution_time_ms: Family<NodeLabels, Histogram>,
    pub soroban_wasm_cache_hits: Family<NodeLabels, Counter>,
    pub soroban_wasm_cache_misses: Family<NodeLabels, Counter>,
    pub soroban_host_function_calls: Family<NodeLabels, Counter>,
    
    // Horizon-specific metrics
    pub horizon_request_duration_seconds: Family<NodeLabels, Histogram>,
    pub horizon_requests_total: Family<NodeLabels, Counter>,
    pub horizon_ingestion_lag_seconds: Family<NodeLabels, Gauge>,
    pub horizon_db_replication_lag_seconds: Family<NodeLabels, Gauge>,
}

impl StellarMetrics {
    /// Create a new StellarMetrics instance and register all metrics
    pub fn new(registry: &mut Registry) -> Arc<Self> {
        let metrics = Arc::new(Self {
            // Ledger metrics
            ledger_close_time_seconds: Family::default(),
            ledger_close_time_p50: Family::default(),
            ledger_close_time_p95: Family::default(),
            ledger_close_time_p99: Family::default(),
            ledger_operations_total: Family::default(),
            ledger_transactions_total: Family::default(),
            ledger_failed_transactions_total: Family::default(),
            
            // Transaction throughput
            transaction_throughput_tps: Family::default(),
            transaction_throughput_ops: Family::default(),
            transaction_apply_time_seconds: Family::default(),
            transaction_queue_size: Family::default(),
            transaction_success_rate: Family::default(),
            
            // Peer connection quality
            peer_connection_count: Family::default(),
            peer_message_latency_ms: Family::default(),
            peer_messages_sent_total: Family::default(),
            peer_messages_received_total: Family::default(),
            peer_connection_errors_total: Family::default(),
            peer_bandwidth_bytes_sent: Family::default(),
            peer_bandwidth_bytes_received: Family::default(),
            peer_connection_uptime_seconds: Family::default(),
            
            // History archive health
            archive_health_status: Family::default(),
            archive_last_check_timestamp: Family::default(),
            archive_check_duration_seconds: Family::default(),
            archive_missing_files_total: Family::default(),
            archive_download_errors_total: Family::default(),
            archive_upload_errors_total: Family::default(),
            archive_size_bytes: Family::default(),
            
            // Database metrics
            database_size_bytes: Family::default(),
            database_growth_rate_bytes_per_hour: Family::default(),
            database_query_duration_seconds: Family::default(),
            database_connection_pool_active: Family::default(),
            database_connection_pool_idle: Family::default(),
            database_transaction_count: Family::default(),
            database_slow_queries_total: Family::default(),
            
            // SCP metrics
            scp_nomination_time_seconds: Family::default(),
            scp_ballot_prepare_time_seconds: Family::default(),
            scp_ballot_commit_time_seconds: Family::default(),
            scp_externalize_time_seconds: Family::default(),
            scp_quorum_intersection_failures: Family::default(),
            
            // Soroban metrics
            soroban_contract_invocations_total: Family::default(),
            soroban_contract_execution_time_ms: Family::default(),
            soroban_wasm_cache_hits: Family::default(),
            soroban_wasm_cache_misses: Family::default(),
            soroban_host_function_calls: Family::default(),
            
            // Horizon metrics
            horizon_request_duration_seconds: Family::default(),
            horizon_requests_total: Family::default(),
            horizon_ingestion_lag_seconds: Family::default(),
            horizon_db_replication_lag_seconds: Family::default(),
        });

        // Register all metrics with descriptions
        registry.register(
            "stellar_ledger_close_time_seconds",
            "Time taken to close a ledger",
            metrics.ledger_close_time_seconds.clone(),
        );
        
        registry.register(
            "stellar_ledger_close_time_p50",
            "50th percentile ledger close time",
            metrics.ledger_close_time_p50.clone(),
        );
        
        registry.register(
            "stellar_ledger_close_time_p95",
            "95th percentile ledger close time",
            metrics.ledger_close_time_p95.clone(),
        );
        
        registry.register(
            "stellar_ledger_close_time_p99",
            "99th percentile ledger close time",
            metrics.ledger_close_time_p99.clone(),
        );
        
        registry.register(
            "stellar_ledger_operations_total",
            "Total number of operations in ledgers",
            metrics.ledger_operations_total.clone(),
        );
        
        registry.register(
            "stellar_ledger_transactions_total",
            "Total number of transactions in ledgers",
            metrics.ledger_transactions_total.clone(),
        );
        
        registry.register(
            "stellar_ledger_failed_transactions_total",
            "Total number of failed transactions",
            metrics.ledger_failed_transactions_total.clone(),
        );
        
        registry.register(
            "stellar_transaction_throughput_tps",
            "Current transaction throughput in transactions per second",
            metrics.transaction_throughput_tps.clone(),
        );
        
        registry.register(
            "stellar_transaction_throughput_ops",
            "Current operation throughput in operations per second",
            metrics.transaction_throughput_ops.clone(),
        );
        
        registry.register(
            "stellar_transaction_apply_time_seconds",
            "Time taken to apply a transaction",
            metrics.transaction_apply_time_seconds.clone(),
        );
        
        registry.register(
            "stellar_transaction_queue_size",
            "Number of transactions waiting in queue",
            metrics.transaction_queue_size.clone(),
        );
        
        registry.register(
            "stellar_transaction_success_rate",
            "Ratio of successful transactions (0.0-1.0)",
            metrics.transaction_success_rate.clone(),
        );
        
        registry.register(
            "stellar_peer_connection_count",
            "Number of peer connections by state",
            metrics.peer_connection_count.clone(),
        );
        
        registry.register(
            "stellar_peer_message_latency_ms",
            "Peer message round-trip latency in milliseconds",
            metrics.peer_message_latency_ms.clone(),
        );
        
        registry.register(
            "stellar_peer_messages_sent_total",
            "Total messages sent to peers",
            metrics.peer_messages_sent_total.clone(),
        );
        
        registry.register(
            "stellar_peer_messages_received_total",
            "Total messages received from peers",
            metrics.peer_messages_received_total.clone(),
        );
        
        registry.register(
            "stellar_peer_connection_errors_total",
            "Total peer connection errors",
            metrics.peer_connection_errors_total.clone(),
        );
        
        registry.register(
            "stellar_peer_bandwidth_bytes_sent",
            "Total bytes sent to peers",
            metrics.peer_bandwidth_bytes_sent.clone(),
        );
        
        registry.register(
            "stellar_peer_bandwidth_bytes_received",
            "Total bytes received from peers",
            metrics.peer_bandwidth_bytes_received.clone(),
        );
        
        registry.register(
            "stellar_peer_connection_uptime_seconds",
            "Peer connection uptime in seconds",
            metrics.peer_connection_uptime_seconds.clone(),
        );
        
        registry.register(
            "stellar_archive_health_status",
            "History archive health status (1=healthy, 0=unhealthy)",
            metrics.archive_health_status.clone(),
        );
        
        registry.register(
            "stellar_archive_last_check_timestamp",
            "Unix timestamp of last archive health check",
            metrics.archive_last_check_timestamp.clone(),
        );
        
        registry.register(
            "stellar_archive_check_duration_seconds",
            "Duration of archive health check",
            metrics.archive_check_duration_seconds.clone(),
        );
        
        registry.register(
            "stellar_archive_missing_files_total",
            "Total number of missing files in archive",
            metrics.archive_missing_files_total.clone(),
        );
        
        registry.register(
            "stellar_archive_download_errors_total",
            "Total archive download errors",
            metrics.archive_download_errors_total.clone(),
        );
        
        registry.register(
            "stellar_archive_upload_errors_total",
            "Total archive upload errors",
            metrics.archive_upload_errors_total.clone(),
        );
        
        registry.register(
            "stellar_archive_size_bytes",
            "Total size of history archive in bytes",
            metrics.archive_size_bytes.clone(),
        );
        
        registry.register(
            "stellar_database_size_bytes",
            "Database size in bytes",
            metrics.database_size_bytes.clone(),
        );
        
        registry.register(
            "stellar_database_growth_rate_bytes_per_hour",
            "Database growth rate in bytes per hour",
            metrics.database_growth_rate_bytes_per_hour.clone(),
        );
        
        registry.register(
            "stellar_database_query_duration_seconds",
            "Database query execution time",
            metrics.database_query_duration_seconds.clone(),
        );
        
        registry.register(
            "stellar_database_connection_pool_active",
            "Number of active database connections",
            metrics.database_connection_pool_active.clone(),
        );
        
        registry.register(
            "stellar_database_connection_pool_idle",
            "Number of idle database connections",
            metrics.database_connection_pool_idle.clone(),
        );
        
        registry.register(
            "stellar_database_transaction_count",
            "Total database transactions",
            metrics.database_transaction_count.clone(),
        );
        
        registry.register(
            "stellar_database_slow_queries_total",
            "Total number of slow queries (>1s)",
            metrics.database_slow_queries_total.clone(),
        );
        
        registry.register(
            "stellar_scp_nomination_time_seconds",
            "Time spent in SCP nomination phase",
            metrics.scp_nomination_time_seconds.clone(),
        );
        
        registry.register(
            "stellar_scp_ballot_prepare_time_seconds",
            "Time spent in SCP ballot prepare phase",
            metrics.scp_ballot_prepare_time_seconds.clone(),
        );
        
        registry.register(
            "stellar_scp_ballot_commit_time_seconds",
            "Time spent in SCP ballot commit phase",
            metrics.scp_ballot_commit_time_seconds.clone(),
        );
        
        registry.register(
            "stellar_scp_externalize_time_seconds",
            "Time spent in SCP externalize phase",
            metrics.scp_externalize_time_seconds.clone(),
        );
        
        registry.register(
            "stellar_scp_quorum_intersection_failures",
            "Number of quorum intersection failures",
            metrics.scp_quorum_intersection_failures.clone(),
        );
        
        registry.register(
            "stellar_soroban_contract_invocations_total",
            "Total Soroban contract invocations",
            metrics.soroban_contract_invocations_total.clone(),
        );
        
        registry.register(
            "stellar_soroban_contract_execution_time_ms",
            "Soroban contract execution time in milliseconds",
            metrics.soroban_contract_execution_time_ms.clone(),
        );
        
        registry.register(
            "stellar_soroban_wasm_cache_hits",
            "Soroban WASM cache hits",
            metrics.soroban_wasm_cache_hits.clone(),
        );
        
        registry.register(
            "stellar_soroban_wasm_cache_misses",
            "Soroban WASM cache misses",
            metrics.soroban_wasm_cache_misses.clone(),
        );
        
        registry.register(
            "stellar_soroban_host_function_calls",
            "Total Soroban host function calls",
            metrics.soroban_host_function_calls.clone(),
        );
        
        registry.register(
            "stellar_horizon_request_duration_seconds",
            "Horizon API request duration",
            metrics.horizon_request_duration_seconds.clone(),
        );
        
        registry.register(
            "stellar_horizon_requests_total",
            "Total Horizon API requests",
            metrics.horizon_requests_total.clone(),
        );
        
        registry.register(
            "stellar_horizon_ingestion_lag_seconds",
            "Horizon ingestion lag behind network",
            metrics.horizon_ingestion_lag_seconds.clone(),
        );
        
        registry.register(
            "stellar_horizon_db_replication_lag_seconds",
            "Horizon database replication lag",
            metrics.horizon_db_replication_lag_seconds.clone(),
        );

        metrics
    }
}
