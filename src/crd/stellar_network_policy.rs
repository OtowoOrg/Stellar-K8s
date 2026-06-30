//! Networking Policy CRDs for microsegmentation and zero-trust networking
//!
//! This module defines the Custom Resource Definitions for advanced networking policies
//! that enable identity-based microsegmentation, zero-trust architecture, and automatic
//! policy discovery.
//!
//! # Key CRDs
//!
//! - [`StellarNetworkPolicy`] - Identity-based network policies with L7 support
//! - [`StellarNetworkSegment`] - Security zones for pod grouping
//! - [`StellarWorkloadProfile`] - Workload identity and behavioral profiles

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// StellarNetworkPolicy defines network access policies for pods with support for
/// L3/L4/L7 rules, identity-based selectors, and automatic policy generation.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarNetworkPolicy",
    namespaced
)]
#[kube(status = "StellarNetworkPolicyStatus")]
pub struct StellarNetworkPolicySpec {
    /// Pod selector indicating which pods this policy applies to
    pub selector: LabelSelector,

    /// Ingress rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<Vec<IngressRule>>,

    /// Egress rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress: Option<Vec<EgressRule>>,

    /// Policy priority (lower number = higher priority, range: 0-1000)
    #[serde(default)]
    pub priority: i32,

    /// Enable enforcement (true) or warning-only mode (false)
    #[serde(default = "default_enforcement_enabled")]
    pub enforce: bool,

    /// Audit all traffic matching this policy
    #[serde(default)]
    pub audit: bool,

    /// Description of the policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Labels for organizing policies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,
}

/// StellarNetworkPolicyStatus tracks policy status and violations
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct StellarNetworkPolicyStatus {
    /// Current phase: Pending, Active, Error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,

    /// Number of pods covered by this policy
    #[serde(default)]
    pub pods_covered: i32,

    /// Number of violations detected
    #[serde(default)]
    pub violations_detected: i32,

    /// Last update timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Error message if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Conditions for policy status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<Condition>>,
}

/// IngressRule describes traffic allowed to pods matching the selector
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct IngressRule {
    /// From defines where traffic is allowed from
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Vec<NetworkPolicyPeer>>,

    /// Ports define which ports are allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<NetworkPolicyPort>>,

    /// L7Rules define application-layer rules (HTTP, gRPC, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l7Rules: Option<Vec<L7Rule>>,
}

/// EgressRule describes traffic allowed from pods matching the selector
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct EgressRule {
    /// To defines where traffic is allowed to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<NetworkPolicyPeer>>,

    /// Ports define which ports are allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<NetworkPolicyPort>>,

    /// L7Rules define application-layer rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l7Rules: Option<Vec<L7Rule>>,
}

/// NetworkPolicyPeer describes entities that can be selected for traffic rules
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct NetworkPolicyPeer {
    /// PodSelector selects pods by label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub podSelector: Option<LabelSelector>,

    /// NamespaceSelector selects namespaces by label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespaceSelector: Option<LabelSelector>,

    /// SegmentSelector selects a network segment by name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmentSelector: Option<SegmentSelector>,

    /// IPBlock allows traffic from/to specific IP ranges
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipBlock: Option<IPBlock>,
}

/// SegmentSelector refers to a StellarNetworkSegment by name
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct SegmentSelector {
    /// Name of the segment
    pub name: String,
}

/// IPBlock describes a set of IP ranges
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct IPBlock {
    /// CIDR block (e.g., "10.0.0.0/8")
    pub cidr: String,

    /// Except defines exceptions to the CIDR
    #[serde(skip_serializing_if = "Option::is_none")]
    pub except: Option<Vec<String>>,
}

/// NetworkPolicyPort describes a port to allow
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct NetworkPolicyPort {
    /// Protocol (TCP, UDP, SCTP)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,

    /// Port number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<i32>,

    /// Port name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portName: Option<String>,
}

/// L7Rule describes application-layer policy rules
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct L7Rule {
    /// HTTP rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<Vec<HTTPRule>>,

    /// gRPC rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grpc: Option<Vec<GRPCRule>>,

    /// TLS/SNI rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<Vec<TLSRule>>,

    /// DNS rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns: Option<Vec<DNSRule>>,
}

/// HTTPRule describes HTTP-specific policy rules
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct HTTPRule {
    /// Host header match (e.g., "api.stellar.org")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,

    /// HTTP path match (e.g., "/api/v1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// HTTP method (GET, POST, PUT, DELETE, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,

    /// HTTP header matches
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<HeaderMatch>>,
}

/// HeaderMatch describes HTTP header matching
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct HeaderMatch {
    /// Header name
    pub name: String,

    /// Exact header value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Regex header value match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
}

/// GRPCRule describes gRPC-specific policy rules
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct GRPCRule {
    /// Fully qualified method name (e.g., "/stellar.Validator/GetStatus")
    pub method: String,

    /// Metadata matches for gRPC headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Vec<MetadataMatch>>,
}

/// MetadataMatch describes gRPC metadata matching
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct MetadataMatch {
    /// Metadata key
    pub name: String,

    /// Exact value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Regex value match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
}

/// TLSRule describes TLS/SNI-specific policy rules
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct TLSRule {
    /// SNI (Server Name Indication) match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sni: Option<String>,

    /// Cipher suites to allow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cipherSuites: Option<Vec<String>>,
}

/// DNSRule describes DNS-specific policy rules
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct DNSRule {
    /// Domain name or pattern (e.g., "*.stellar.org")
    pub domain: String,

    /// Record type (A, AAAA, MX, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recordType: Option<String>,
}

/// StellarNetworkSegment defines a security zone/segment
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarNetworkSegment",
    namespaced
)]
#[kube(status = "StellarNetworkSegmentStatus")]
pub struct StellarNetworkSegmentSpec {
    /// Pod selector indicating which pods belong to this segment
    pub podSelector: LabelSelector,

    /// Description of the segment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Default ingress rules for all pods in this segment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaultIngress: Option<Vec<IngressRule>>,

    /// Default egress rules for all pods in this segment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaultEgress: Option<Vec<EgressRule>>,

    /// Parent segment (for hierarchy)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parentSegment: Option<String>,

    /// Tier level (critical, standard, public, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,

    /// Labels for organizing segments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,
}

/// StellarNetworkSegmentStatus tracks segment status
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct StellarNetworkSegmentStatus {
    /// Number of pods in this segment
    #[serde(default)]
    pub pod_count: i32,

    /// Number of policies applied
    #[serde(default)]
    pub policy_count: i32,

    /// Last update timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

/// StellarWorkloadProfile describes a workload's identity and behavioral patterns
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarWorkloadProfile",
    namespaced
)]
pub struct StellarWorkloadProfileSpec {
    /// Pod selector for this profile
    pub podSelector: LabelSelector,

    /// Workload identity information
    pub identity: WorkloadIdentity,

    /// Allowed destinations (learned from flows)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowedDestinations: Option<Vec<AllowedDestination>>,

    /// Threat level assessment (low, medium, high, critical)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threatLevel: Option<String>,

    /// Last updated timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastUpdated: Option<String>,

    /// Learning mode enabled (true = learning, false = enforcing)
    #[serde(default = "default_learning_mode")]
    pub learning: bool,
}

/// WorkloadIdentity describes the identity of a workload
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct WorkloadIdentity {
    /// Service account name
    pub serviceAccount: String,

    /// Custom identity labels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,

    /// SPIFFE identity (URI format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spiffeId: Option<String>,

    /// Certificate thumbprint for mTLS
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificateThumbprint: Option<String>,
}

/// AllowedDestination describes a destination allowed by the workload profile
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct AllowedDestination {
    /// Destination target (pod:label, service:name, external:ip, etc.)
    pub target: String,

    /// Allowed ports
    pub ports: Vec<i32>,

    /// Allowed protocols (TCP, UDP, ICMP, etc.)
    pub protocols: Vec<String>,

    /// Confidence score (0.0-1.0) from learning
    #[serde(default)]
    pub confidence: f64,

    /// First observed timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firstSeen: Option<String>,

    /// Last observed timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastSeen: Option<String>,

    /// Number of observations
    #[serde(default)]
    pub observationCount: i32,
}

/// LabelSelector represents a Kubernetes label selector
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct LabelSelector {
    /// Key-value label matches
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matchLabels: Option<BTreeMap<String, String>>,

    /// Expression-based label matches
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matchExpressions: Option<Vec<LabelSelectorRequirement>>,
}

/// LabelSelectorRequirement describes a single label selector requirement
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct LabelSelectorRequirement {
    /// Label key
    pub key: String,

    /// Operator (In, NotIn, Exists, DoesNotExist, Gt, Lt)
    pub operator: String,

    /// Values for the label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}

/// Condition represents a condition status
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Condition {
    /// Condition type
    pub r#type: String,

    /// Condition status (True, False, Unknown)
    pub status: String,

    /// Last transition time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastTransitionTime: Option<String>,

    /// Reason for the condition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Message describing the condition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn default_enforcement_enabled() -> bool {
    true
}

fn default_learning_mode() -> bool {
    false
}
