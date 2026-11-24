# Design: Advanced Networking Policies with Microsegmentation

## Component Architecture

### Core Components

```
┌─────────────────────────────────────────────────────────────┐
│                  Stellar-K8s Operator                        │
│                                                               │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ scheduler/networking/mod.rs                          │   │
│  │ - Reconcilers for network CRDs                       │   │
│  │ - Policy controller                                  │   │
│  │ - Enforcement layer                                  │   │
│  └──────────────────────────────────────────────────────┘   │
│                           │                                   │
│  ┌──────────────────┐    ▼     ┌──────────────────┐         │
│  │ CRD Definitions  │  ┌─────────────────────┐   │ Policy   │
│  │                  │  │ Network Policy      │   │ Engine   │
│  │ • Stellar        │  │ Generators          │   │          │
│  │   NetworkPolicy  │  │                     │   │ Enhanced │
│  │ • Stellar        │  │ 1. NetworkPolicy    │   │ with ML  │
│  │   NetworkSegment │  │    (L3/L4)          │   │          │
│  │ • Stellar        │  │ 2. Cilium Policy    │   │          │
│  │   AccessPolicy   │  │    (L7)             │   │          │
│  │ • Stellar        │  │ 3. Calico Policy    │   │          │
│  │   WorkloadProfile│  │    (Global)         │   │          │
│  │                  │  └─────────────────────┘   │          │
│  └──────────────────┘           │                └──────────┘
│         │                        ▼                │
│         │          ┌──────────────────────────┐   │
│         │          │ Flow Analysis & Policy    │   │
│         └─────────▶│ Discovery                │   │
│                    │ • Behavioral baseline    │   │
│                    │ • Anomaly detection      │   │
│                    │ • Policy recommendation  │   │
│                    └──────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
         │                           │                │
         └───────────────┬───────────┴────────────────┘
                         │
           ┌─────────────┼─────────────┐
           ▼             ▼             ▼
      ┌─────────┐   ┌─────────┐   ┌─────────┐
      │Kubernetes│   │ Cilium  │   │ Calico  │
      │NetworkPo-│   │Policy   │   │Policy   │
      │licy      │   │(L7,DNS) │   │(Global) │
      │(L3/L4)   │   │         │   │         │
      └─────────┘   └─────────┘   └─────────┘
           │             │             │
           └─────────────┼─────────────┘
                         ▼
                  ┌──────────────┐
                  │  Data Plane  │
                  │  Enforcement │
                  └──────────────┘
```

## Module Structure

```rust
// src/scheduler/networking/

pub mod mod.rs              // Main module & reconciliation
pub mod crd.rs              // CRD definitions
pub mod policy_generator.rs // Policy generation logic
pub mod segment.rs          // Segment management
pub mod identity.rs         // Workload identity binding
pub mod flow_analyzer.rs    // Flow analysis & discovery
pub mod enforcement.rs      // Policy enforcement layer
pub mod cni.rs              // CNI abstraction layer
pub mod metrics.rs          // Prometheus metrics
pub mod audit.rs            // Audit logging
pub mod compliance.rs       // Compliance reports
```

## CRD Definitions

### 1. StellarNetworkPolicy CRD

```rust
// src/scheduler/networking/crd.rs

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(group = "stellar.org", version = "v1alpha1", kind = "StellarNetworkPolicy")]
#[kube(namespaced)]
pub struct StellarNetworkPolicySpec {
    /// Pod selector for this policy
    pub selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector,
    
    /// Ingress rules
    pub ingress: Option<Vec<IngressRule>>,
    
    /// Egress rules
    pub egress: Option<Vec<EgressRule>>,
    
    /// Policy priority (lower = higher priority)
    #[serde(default)]
    pub priority: i32,
    
    /// Enable enforcement (true) or warning-only mode (false)
    #[serde(default = "default_enforcement_enabled")]
    pub enforce: bool,
    
    /// Audit this policy
    #[serde(default)]
    pub audit: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct IngressRule {
    pub from: Option<Vec<NetworkPolicyPeer>>,
    pub ports: Option<Vec<NetworkPolicyPort>>,
    pub l7Rules: Option<Vec<L7Rule>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct EgressRule {
    pub to: Option<Vec<NetworkPolicyPeer>>,
    pub ports: Option<Vec<NetworkPolicyPort>>,
    pub l7Rules: Option<Vec<L7Rule>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct NetworkPolicyPeer {
    pub podSelector: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector>,
    pub namespaceSelector: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector>,
    pub segmentSelector: Option<SegmentSelector>,
    pub ipBlock: Option<IPBlock>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct SegmentSelector {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct IPBlock {
    pub cidr: String,
    pub except: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct NetworkPolicyPort {
    pub protocol: Option<String>,
    pub port: Option<i32>,
    pub portName: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct L7Rule {
    pub http: Option<Vec<HTTPRule>>,
    pub grpc: Option<Vec<GRPCRule>>,
    pub tls: Option<Vec<TLSRule>>,
    pub dns: Option<Vec<DNSRule>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct HTTPRule {
    pub host: Option<String>,
    pub path: Option<String>,
    pub method: Option<String>,
    pub header: Option<Vec<HeaderMatch>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct HeaderMatch {
    pub name: String,
    pub value: Option<String>,
    pub regex: Option<String>,
}

// Similar for GRPCRule, TLSRule, DNSRule...

fn default_enforcement_enabled() -> bool {
    true
}
```

### 2. StellarNetworkSegment CRD

```rust
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(group = "stellar.org", version = "v1alpha1", kind = "StellarNetworkSegment")]
#[kube(namespaced)]
pub struct StellarNetworkSegmentSpec {
    /// Pods in this segment
    pub podSelector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector,
    
    /// Description
    pub description: Option<String>,
    
    /// Default ingress rules for segment
    pub defaultIngress: Option<Vec<IngressRule>>,
    
    /// Default egress rules for segment
    pub defaultEgress: Option<Vec<EgressRule>>,
    
    /// Parent segment (for hierarchy)
    pub parentSegment: Option<String>,
    
    /// Tier (e.g., "critical", "standard", "public")
    pub tier: Option<String>,
}
```

### 3. StellarWorkloadProfile CRD

```rust
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(group = "stellar.org", version = "v1alpha1", kind = "StellarWorkloadProfile")]
#[kube(namespaced)]
pub struct StellarWorkloadProfileSpec {
    /// Pod this profile describes
    pub podSelector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector,
    
    /// Workload identity
    pub identity: WorkloadIdentity,
    
    /// Allowed destinations (learned from flows)
    pub allowedDestinations: Vec<AllowedDestination>,
    
    /// Threat level (low, medium, high)
    pub threatLevel: Option<String>,
    
    /// Last updated timestamp
    pub lastUpdated: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct WorkloadIdentity {
    pub serviceAccount: String,
    pub labels: Option<BTreeMap<String, String>>,
    pub spiffeId: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct AllowedDestination {
    pub target: String,     // "pod:label", "service:name", "external:ip"
    pub ports: Vec<i32>,
    pub protocols: Vec<String>,
    pub confidence: f64,    // 0.0-1.0
    pub firstSeen: String,
    pub lastSeen: String,
}
```

## Policy Generator Implementation

```rust
// src/scheduler/networking/policy_generator.rs

use crate::crd::{StellarNetworkPolicy, StellarNetworkSegment};
use k8s_openapi::api::networking::v1::{NetworkPolicy, NetworkPolicySpec};

pub struct PolicyGenerator {
    cni: CNIType,
    namespace: String,
}

impl PolicyGenerator {
    pub async fn generate_kubernetes_policy(
        &self,
        sn_policy: &StellarNetworkPolicy,
    ) -> Result<NetworkPolicy> {
        // Convert StellarNetworkPolicy → Kubernetes NetworkPolicy
        // Handle L3/L4 rules only (L7 requires Cilium)
        let mut np = NetworkPolicy::default();
        np.metadata.name = Some(sn_policy.metadata.name.clone());
        np.metadata.namespace = sn_policy.metadata.namespace.clone();
        
        // Build spec from StellarNetworkPolicy
        np.spec = Some(self.build_network_policy_spec(sn_policy)?);
        
        Ok(np)
    }
    
    pub async fn generate_cilium_policy(
        &self,
        sn_policy: &StellarNetworkPolicy,
    ) -> Result<String> {
        // Generate Cilium CiliumNetworkPolicy YAML
        // Supports L7 rules, DNS, etc.
        todo!()
    }
    
    pub async fn generate_calico_policy(
        &self,
        sn_policy: &StellarNetworkPolicy,
    ) -> Result<String> {
        // Generate Calico GlobalNetworkPolicy YAML
        todo!()
    }
    
    pub async fn generate_segment_policies(
        &self,
        segment: &StellarNetworkSegment,
        other_segments: &[StellarNetworkSegment],
    ) -> Result<Vec<StellarNetworkPolicy>> {
        // Generate inter-segment policies
        // For each other segment, create allow/deny rules
        let mut policies = Vec::new();
        
        for other in other_segments {
            let policy = self.create_segment_boundary_policy(segment, other)?;
            policies.push(policy);
        }
        
        Ok(policies)
    }
    
    fn build_network_policy_spec(
        &self,
        sn_policy: &StellarNetworkPolicy,
    ) -> Result<NetworkPolicySpec> {
        // Implementation
        todo!()
    }
    
    fn create_segment_boundary_policy(
        &self,
        from: &StellarNetworkSegment,
        to: &StellarNetworkSegment,
    ) -> Result<StellarNetworkPolicy> {
        // Implementation
        todo!()
    }
}
```

## Flow Analysis & Policy Discovery

```rust
// src/scheduler/networking/flow_analyzer.rs

use crate::network_observability::{NetworkFlow, FlowStore};
use std::collections::BTreeMap;

pub struct PolicyDiscoveryEngine {
    flow_store: Arc<FlowStore>,
    learning_duration: Duration,
    anomaly_threshold: f64,
}

impl PolicyDiscoveryEngine {
    pub async fn learn_baseline(&mut self) -> Result<()> {
        // Run for configured duration (default 7 days)
        let flows = self.flow_store.get_all_flows().await?;
        
        // Build baseline: pod_identity → allowed destinations
        self.baseline = self.build_baseline(&flows)?;
        
        Ok(())
    }
    
    pub async fn detect_anomalies(&self) -> Result<Vec<FlowAnomaly>> {
        let recent_flows = self.flow_store.get_recent_flows(
            Duration::from_secs(300), // Last 5 minutes
        ).await?;
        
        let mut anomalies = Vec::new();
        for flow in recent_flows {
            if !self.is_in_baseline(&flow) {
                anomalies.push(FlowAnomaly {
                    flow,
                    reason: AnomalyReason::OutsideBaseline,
                    severity: self.calculate_severity(&flow),
                });
            }
        }
        
        Ok(anomalies)
    }
    
    pub async fn generate_policies_from_baseline(
        &self,
    ) -> Result<Vec<StellarNetworkPolicy>> {
        // Convert baseline → policies
        let mut policies = Vec::new();
        
        for (src_identity, destinations) in &self.baseline {
            let policy = self.create_policy_for_identity(
                src_identity,
                destinations,
            )?;
            policies.push(policy);
        }
        
        Ok(policies)
    }
    
    fn build_baseline(
        &self,
        flows: &[NetworkFlow],
    ) -> Result<BTreeMap<WorkloadIdentity, Vec<AllowedDestination>>> {
        let mut baseline = BTreeMap::new();
        
        for flow in flows {
            let src_identity = self.extract_identity(&flow.src_pod)?;
            let dst = AllowedDestination {
                target: flow.dst_pod.clone().unwrap_or_else(|| flow.dst_ip.clone()),
                ports: vec![flow.dst_port as i32],
                protocols: vec![flow.protocol.clone()],
                confidence: 0.95, // From baseline
                firstSeen: flow.timestamp.to_rfc3339(),
                lastSeen: flow.timestamp.to_rfc3339(),
            };
            
            baseline.entry(src_identity)
                .or_insert_with(Vec::new)
                .push(dst);
        }
        
        Ok(baseline)
    }
    
    fn extract_identity(&self, pod_name: &str) -> Result<WorkloadIdentity> {
        // Query pod metadata, get ServiceAccount, labels, etc.
        todo!()
    }
}
```

## Reconciliation Loop

```rust
// src/scheduler/networking/mod.rs

use kube::runtime::controller::{Action, Controller};

pub async fn reconcile_network_policy(
    policy: Arc<StellarNetworkPolicy>,
    ctx: Arc<ControllerState>,
) -> Result<Action> {
    let client = ctx.client.clone();
    let namespace = policy.metadata.namespace.clone().unwrap_or_default();
    
    // 1. Generate platform-specific policies
    let generator = PolicyGenerator::new(ctx.cni_type, namespace);
    
    let k8s_policy = generator.generate_kubernetes_policy(&policy).await?;
    let client_ns: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    client_ns.apply(&k8s_policy, &PostParams::default()).await?;
    
    if ctx.cni_type == CNIType::Cilium {
        let cilium_policy_yaml = generator.generate_cilium_policy(&policy).await?;
        // Apply Cilium policy via kubectl apply
        apply_cilium_policy(&client, &namespace, &cilium_policy_yaml).await?;
    }
    
    if ctx.cni_type == CNIType::Calico {
        let calico_policy_yaml = generator.generate_calico_policy(&policy).await?;
        apply_calico_policy(&client, &namespace, &calico_policy_yaml).await?;
    }
    
    // 2. Create audit entry
    audit_log(&policy, "Applied").await?;
    
    // 3. Update status
    update_policy_status(&client, &policy, PolicyStatus::Applied).await?;
    
    Ok(Action::requeue(Duration::from_secs(300)))
}
```

## CNI Abstraction Layer

```rust
// src/scheduler/networking/cni.rs

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CNIType {
    Kubernetes,  // NetworkPolicy only
    Cilium,      // L7, DNS, mTLS
    Calico,      // Global policies
    Antrea,      // Layer 7
}

pub trait CNIAdapter: Send + Sync {
    async fn detect_cni(&self, client: &Client) -> Result<CNIType>;
    async fn apply_policy(&self, policy: &str) -> Result<()>;
    async fn delete_policy(&self, name: &str, namespace: &str) -> Result<()>;
    async fn supports_l7(&self) -> bool;
    async fn supports_dns(&self) -> bool;
}

pub struct KubernetesAdapter;
pub struct CiliumAdapter;
pub struct CalicoAdapter;
pub struct AntreaAdapter;

impl CNIAdapter for CiliumAdapter {
    async fn detect_cni(&self, client: &Client) -> Result<CNIType> {
        // Check for Cilium DaemonSet in kube-system
        let api: Api<DaemonSet> = Api::namespaced(client.clone(), "kube-system");
        match api.get("cilium").await {
            Ok(_) => Ok(CNIType::Cilium),
            Err(_) => Err("Cilium not detected".into()),
        }
    }
    
    async fn supports_l7(&self) -> bool {
        true
    }
    
    async fn supports_dns(&self) -> bool {
        true
    }
    
    // ... implementation
}
```

## Metrics & Observability

```rust
// src/scheduler/networking/metrics.rs

use prometheus::{Counter, Gauge, Histogram};

pub struct NetworkingMetrics {
    pub policies_applied: Counter,
    pub policy_violations: Counter,
    pub policy_enforcement_latency: Histogram,
    pub pods_covered: Gauge,
    pub flows_captured: Counter,
    pub anomalies_detected: Counter,
}

impl NetworkingMetrics {
    pub fn new() -> Self {
        Self {
            policies_applied: Counter::new("stellar_network_policies_applied", 
                "Total policies applied").unwrap(),
            policy_violations: Counter::new("stellar_network_policy_violations_total",
                "Total policy violations").unwrap(),
            policy_enforcement_latency: Histogram::new("stellar_network_policy_latency_ms",
                "Policy enforcement latency in ms").unwrap(),
            pods_covered: Gauge::new("stellar_network_pods_covered",
                "Number of pods with policies").unwrap(),
            flows_captured: Counter::new("stellar_network_flows_captured",
                "Total flows captured").unwrap(),
            anomalies_detected: Counter::new("stellar_network_anomalies_detected",
                "Total anomalies detected").unwrap(),
        }
    }
}
```

## Testing Strategy

```rust
// src/scheduler/networking/tests/

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_policy_generation() {
        let policy = create_test_policy();
        let generator = PolicyGenerator::new(CNIType::Kubernetes, "default");
        
        let k8s_policy = generator.generate_kubernetes_policy(&policy).await.unwrap();
        assert!(k8s_policy.spec.is_some());
    }
    
    #[tokio::test]
    async fn test_segment_policies() {
        // Test inter-segment policy generation
    }
    
    #[tokio::test]
    async fn test_l7_policy_generation() {
        // Test L7 rule handling
    }
    
    #[tokio::test]
    async fn test_cni_detection() {
        // Test CNI type detection
    }
}
```

## Integration Points

### With Network Observability
- Use `FlowStore` and `FlowAnalyzer` for baseline learning
- Leverage `SecurityMonitor` for violation detection
- Integrate `TopologyGraph` for visualization

### With Identity Module
- Use identity types for policy binding
- Integrate SPIFFE/SVID support
- Leverage ABAC engine for decisions

### With Compliance Engine
- Report on network segmentation compliance
- Track policy violations for audit trail
- Generate compliance reports

### With Service Mesh (#875)
- Generate Istio `AuthorizationPolicy` resources
- Support mTLS enforcement
- L7 policy generation through service mesh

## Performance Considerations

- **Policy Caching**: Cache compiled policies at node level
- **BPF Optimization**: Use eBPF programs for fast enforcement
- **Flow Sampling**: Configurable sampling rate for high-volume clusters
- **Policy Consolidation**: Merge overlapping rules to reduce rule count

## Rollout Strategy

1. **Phase 1** (Week 1-2): CRDs + basic policy generation
2. **Phase 2** (Week 3-4): Flow analysis + baseline learning
3. **Phase 3** (Week 5-6): CNI integration + enforcement
4. **Phase 4** (Week 7-8): L7 + DNS policies
5. **Phase 5** (Week 9-10): Compliance reports + dashboards
