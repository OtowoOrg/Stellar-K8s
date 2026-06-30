//! ChaosExperiment Custom Resource Definition
//!
//! Defines chaos experiments for testing system resilience through
//! automated fault injection and recovery validation.

use chrono::{DateTime, Utc};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// ChaosExperiment CRD - defines a chaos experiment
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "chaos.stellar.org",
    version = "v1alpha1",
    kind = "ChaosExperiment",
    namespaced,
    status = "ChaosExperimentStatus",
    shortname = "chaos",
    printcolumn = r#"{"name":"Status","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ChaosExperimentSpec {
    /// Description of the experiment
    pub description: String,

    /// Service name to target (e.g., stellar-node, horizon)
    pub target_service: String,

    /// Faults to inject
    pub faults: Vec<FaultSpec>,

    /// Steady-state hypothesis to validate
    #[serde(default)]
    pub steady_state: SteadyStateHypothesis,

    /// Experiment schedule
    #[serde(default)]
    pub schedule: Option<ExperimentSchedule>,

    /// Safety constraints
    #[serde(default)]
    pub safety_constraints: SafetyConstraints,

    /// Blast radius control
    #[serde(default)]
    pub blast_radius: BlastRadiusControl,

    /// Experiment duration
    #[serde(default)]
    pub duration_seconds: Option<u32>,

    /// Maximum number of concurrent faults
    #[serde(default = "default_concurrency")]
    pub max_concurrency: u32,

    /// Rollback strategy on failure
    #[serde(default)]
    pub rollback_on_failure: bool,
}

fn default_concurrency() -> u32 {
    1
}

/// Steady-state hypothesis definition
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SteadyStateHypothesis {
    /// Minimum availability percentage expected
    #[serde(default = "default_minAvailability")]
    pub min_availability_percent: f32,

    /// Maximum error rate allowed
    #[serde(default = "default_maxErrorRate")]
    pub max_error_rate_percent: f32,

    /// Maximum latency increase allowed (ms)
    #[serde(default = "default_maxLatency")]
    pub max_latency_increase_ms: u64,

    /// Probes to validate steady state
    #[serde(default)]
    pub probes: Vec<SteadyStateProbe>,

    /// Duration to wait for steady state before declaring success
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout_seconds: u32,
}

fn default_minAvailability() -> f32 {
    95.0
}

fn default_maxErrorRate() -> f32 {
    5.0
}

fn default_maxLatency() -> u64 {
    1000
}

fn default_probe_timeout() -> u32 {
    30
}

/// Steady-state probe definition
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SteadyStateProbe {
    pub name: String,
    pub probe_type: ProbeType,
    pub endpoint: Option<String>,
    pub command: Option<CommandProbe>,
    pub interval_seconds: u32,
    pub timeout_seconds: u32,
    pub expected_result: Option<String>,
}

#[derive(Clone, Debug, JsonSchema, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeType {
    Http,
    Tcp,
    Command,
    Metric,
}

/// Command probe configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandProbe {
    pub command: Vec<String>,
    pub expected_output: Option<String>,
    pub expected_exit_code: Option<i32>,
}

/// Experiment schedule
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentSchedule {
    /// Cron expression for scheduling
    pub cron: Option<String>,

    /// Interval-based scheduling (in seconds)
    pub interval_seconds: Option<u64>,

    /// Start time
    pub start_time: Option<DateTime<Utc>>,

    /// End time
    pub end_time: Option<DateTime<Utc>>,

    /// Number of times to repeat
    #[serde(default)]
    pub repeat_count: Option<u32>,

    /// Whether to run immediately on creation
    #[serde(default)]
    pub run_immediately: bool,
}

/// Safety constraints for the experiment
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SafetyConstraints {
    /// Maximum number of pods that can be affected
    #[serde(default = "default_max_pod_impact")]
    pub max_pod_impact: u32,

    /// Maximum CPU throttling percentage
    #[serde(default = "default_max_cpu_throttle")]
    pub max_cpu_throttle_percent: u32,

    /// Maximum memory limit percentage
    #[serde(default = "default_max_memory_limit")]
    pub max_memory_limit_percent: u32,

    /// Network partition allowed
    #[serde(default)]
    pub allow_network_partition: bool,

    /// Data corruption simulation allowed
    #[serde(default)]
    pub allow_data_corruption: bool,

    /// Require approval before execution
    #[serde(default)]
    pub require_approval: bool,

    /// Annotations that must be present for execution
    #[serde(default)]
    pub required_annotations: Vec<String>,

    /// Namespaces excluded from experiments
    #[serde(default)]
    pub excluded_namespaces: Vec<String>,
}

fn default_max_pod_impact() -> u32 {
    1
}

fn default_max_cpu_throttle_percent() -> u32 {
    50
}

fn default_max_memory_limit_percent() -> u32 {
    50
}

/// Blast radius control
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlastRadiusControl {
    /// Percentage of pods to affect (0-100)
    #[serde(default = "default_blast_percentage")]
    pub percentage: u8,

    /// Specific pod labels to target
    #[serde(default)]
    pub target_labels: std::collections::HashMap<String, String>,

    /// Specific namespaces to include
    #[serde(default)]
    pub namespaces: Vec<String>,

    /// Do not exceed this number of affected instances
    #[serde(default = "default_max_affected")]
    pub max_affected: u32,

    /// Failure domains (e.g., specific nodes, zones)
    #[serde(default)]
    pub failure_domains: Vec<String>,

    /// Progressive rollout of faults
    #[serde(default)]
    pub progressive_rollout: bool,

    /// Delay between each progressive fault (seconds)
    #[serde(default = "default_progressive_delay")]
    pub progressive_delay_seconds: u32,
}

fn default_blast_percentage() -> u8 {
    10
}

fn default_max_affected() -> u32 {
    3
}

fn default_progressive_delay() -> u32 {
    10
}

/// Fault specification
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultSpec {
    pub name: String,
    pub fault_type: FaultType,
    pub target: FaultTarget,

    /// Fault configuration
    #[serde(default)]
    pub config: FaultConfig,

    /// Duration of the fault
    #[serde(default = "default_fault_duration")]
    pub duration_seconds: u32,

    /// Whether to force the fault
    #[serde(default)]
    pub force: bool,
}

fn default_fault_duration() -> u32 {
    30
}

/// Fault type
#[derive(Clone, Debug, JsonSchema, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FaultType {
    /// Network fault types
    Network(NetworkFault),
    /// CPU fault types
    Cpu(CpuFault),
    /// Memory fault types
    Memory(MemoryFault),
    /// Disk fault types
    Disk(DiskFault),
    /// Pod kill fault
    PodKill,
    /// Container kill fault
    ContainerKill,
    /// DNS fault types
    Dns(DnsFault),
    /// Time travel fault
    ClockSkew,
    /// Kernel panic simulation
    KernelPanic,
    /// AWS-specific faults
    Aws(AwsFault),
    /// GCP-specific faults
    Gcp(GcpFault),
    /// Azure-specific faults
    Azure(AzureFault),
}

/// Network fault configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkFault {
    /// Latency to add (milliseconds)
    #[serde(default)]
    pub latency_ms: Option<u64>,

    /// Packet loss percentage (0-100)
    #[serde(default)]
    pub packet_loss_percent: Option<f32>,

    /// Bandwidth limit (kbps)
    #[serde(default)]
    pub bandwidth_limit_kbps: Option<u64>,

    /// DNS failure
    #[serde(default)]
    pub dns_failure: bool,

    /// DNS to block
    #[serde(default)]
    pub dns_block_list: Option<Vec<String>>,

    /// Connection timeout (seconds)
    #[serde(default)]
    pub connection_timeout_seconds: Option<u32>,

    /// Corrupt packet percentage
    #[serde(default)]
    pub corrupt_percent: Option<f32>,

    /// Duplicate packet percentage
    #[serde(default)]
    pub duplicate_percent: Option<f32>,
}

/// CPU fault configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CpuFault {
    /// Load percentage (0-100)
    #[serde(default)]
    pub load_percent: Option<u8>,

    /// Number of cores to stress
    #[serde(default = "default_cpu_cores")]
    pub cores: u8,

    /// Stress duration
    #[serde(default)]
    pub duration_seconds: Option<u32>,
}

fn default_cpu_cores() -> u8 {
    1
}

/// Memory fault configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFault {
    /// Memory stress type
    pub stress_type: Option<MemoryStressType>,

    /// Memory consumption percentage
    #[serde(default)]
    pub consumption_percent: Option<u8>,

    /// Amount of memory to consume (MB)
    #[serde(default)]
    pub consumption_mb: Option<u64>,

    /// Memory pressure level
    #[serde(default)]
    pub pressure_level: Option<MemoryPressureLevel>,

    /// Duration of memory stress
    #[serde(default)]
    pub duration_seconds: Option<u32>,
}

#[derive(Clone, Debug, JsonSchema, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryStressType {
    Fill,
    Hog,
    Leak,
}

#[derive(Clone, Debug, JsonSchema, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryPressureLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Disk fault configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskFault {
    /// Fill disk to percentage
    #[serde(default)]
    pub fill_percent: Option<u8>,

    /// Read latency to add (ms)
    #[serde(default)]
    pub read_latency_ms: Option<u64>,

    /// Write latency to add (ms)
    #[serde(default)]
    pub write_latency_ms: Option<u64>,

    /// IOPS limit
    #[serde(default)]
    pub iops_limit: Option<u64>,

    /// Directory to target
    #[serde(default)]
    pub target_directory: Option<String>,
}

/// DNS fault configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DnsFault {
    /// Block specific DNS
    #[serde(default)]
    pub block: bool,

    /// DNS to block
    #[serde(default)]
    pub dns_servers: Option<Vec<String>>,

    /// DNS response to inject
    #[serde(default)]
    pub inject_response: Option<String>,

    /// DNS lookup failure
    #[serde(default)]
    pub lookup_failure: bool,
}

/// AWS-specific faults
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AwsFault {
    /// Detach EBS volume
    #[serde(default)]
    pub detach_ebs: bool,

    /// EC2 instance stop
    #[serde(default)]
    pub ec2_stop: bool,

    /// RDS failover
    #[serde(default)]
    pub rds_failover: bool,

    /// Lambda function timeout
    #[serde(default)]
    pub lambda_timeout: bool,

    /// S3 bucket made private
    #[serde(default)]
    pub s3_restrict: bool,
}

/// GCP-specific faults
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GcpFault {
    /// Stop GCE instance
    #[serde(default)]
    pub gce_stop: bool,

    /// Detach persistent disk
    #[serde(default)]
    pub detach_disk: bool,

    /// Cloud SQL failover
    #[serde(default)]
    pub cloudsql_failover: bool,

    /// GKE node pool stop
    #[serde(default)]
    pub gke_stop: bool,
}

/// Azure-specific faults
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AzureFault {
    /// Stop VM
    #[serde(default)]
    pub vm_stop: bool,

    /// Detach disk
    #[serde(default)]
    pub detach_disk: bool,

    /// SQL Database failover
    #[serde(default)]
    pub sql_failover: bool,

    /// AKS node stop
    #[serde(default)]
    pub aks_stop: bool,
}

/// Fault target specification
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultTarget {
    /// Pod labels to match
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,

    /// Namespace to target
    pub namespace: Option<String>,

    /// Pod name pattern (regex)
    #[serde(default)]
    pub pod_pattern: Option<String>,

    /// Container name
    #[serde(default)]
    pub container_name: Option<String>,

    /// Host network
    #[serde(default)]
    pub host_network: bool,
}

/// Fault configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultConfig {
    /// Environment variables to set
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,

    /// Volume mounts
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,

    /// Annotations to add
    #[serde(default)]
    pub annotations: std::collections::HashMap<String, String>,
}

/// Volume mount configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeMount {
    pub name: String,
    pub mount_path: String,
    pub read_only: bool,
}

/// ChaosExperiment status
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChaosExperimentStatus {
    /// Current phase
    #[serde(default)]
    pub phase: ExperimentPhase,

    /// Current fault being executed
    #[serde(default)]
    pub current_fault: Option<String>,

    /// Start time
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,

    /// End time
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,

    /// Probe results
    #[serde(default)]
    pub probe_results: Vec<ProbeResult>,

    /// Experiment results
    #[serde(default)]
    pub results: Option<ExperimentResults>,

    /// Fault history
    #[serde(default)]
    pub fault_history: Vec<FaultExecution>,

    /// Last error message
    #[serde(default)]
    pub last_error: Option<String>,
}

/// Experiment phase
#[derive(Clone, Debug, Default, JsonSchema, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExperimentPhase {
    #[default]
    Pending,
    Scheduled,
    Running,
    Paused,
    VerifyingSteadyState,
    InjectingFault,
    Recovering,
    Completed,
    Failed,
    Cancelled,
}

/// Probe result
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub name: String,
    pub probe_type: String,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
    pub message: Option<String>,
    pub response_time_ms: Option<u64>,
}

/// Experiment results
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentResults {
    /// Whether experiment was successful
    pub success: bool,

    /// Total duration in seconds
    pub duration_seconds: u64,

    /// Number of faults injected
    pub faults_injected: u32,

    /// Number of faults successfully recovered
    pub faults_recovered: u32,

    /// Steady state validation passed
    pub steady_state_validated: bool,

    /// Average probe response time
    pub avg_probe_response_ms: f64,

    /// Error rate during experiment
    pub error_rate_percent: f32,

    /// Availability during experiment
    pub availability_percent: f32,

    /// Resilience score (0-100)
    pub resilience_score: u8,

    /// Findings and observations
    #[serde(default)]
    pub findings: Vec<String>,
}

/// Fault execution record
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultExecution {
    pub name: String,
    pub fault_type: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub success: bool,
    pub error: Option<String>,
    pub affected_resources: Vec<String>,
}