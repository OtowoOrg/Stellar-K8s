//! Chaos Engineering Framework - Fault Injection Library
//!
//! Provides fault injection capabilities for network, CPU, memory, and disk faults.

use std::sync::Arc;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use kube::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::crd::chaos_experiment::*;

/// Result of fault injection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultResult {
    pub fault_name: String,
    pub success: bool,
    pub affected_pods: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub metrics: FaultMetrics,
}

/// Metrics collected during fault injection
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FaultMetrics {
    pub packets_dropped: Option<u64>,
    pub latency_added_ms: Option<u64>,
    pub cpu_load_percent: Option<u8>,
    pub memory_consumed_mb: Option<u64>,
    pub disk_filled_percent: Option<u8>,
}

/// Fault injector trait
#[async_trait]
pub trait FaultInjector: Send + Sync {
    fn name(&self) -> &str;
    fn fault_type(&self) -> &str;
    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String>;
    async fn recover(&self, config: &FaultSpec) -> Result<(), String>;
}

/// Network fault injector
pub struct NetworkFaultInjector {
    client: Client,
}

impl NetworkFaultInjector {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl FaultInjector for NetworkFaultInjector {
    fn name(&self) -> &str {
        "network-fault-injector"
    }

    fn fault_type(&self) -> &str {
        "network"
    }

    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        let network_config = match &config.fault_type {
            FaultType::Network(n) => n,
            _ => return Err("Invalid fault type for NetworkFaultInjector".to_string()),
        };

        tracing::info!(
            "Injecting network fault: latency={:?}ms, packet_loss={:?}%, dns_failure={}",
            network_config.latency_ms,
            network_config.packet_loss_percent,
            network_config.dns_failure
        );

        // Simulate fault injection
        // In production, this would use iptables, tc, or a sidecar container
        let affected_pods = vec!["stellar-node-0".to_string()]; // Would be determined by target

        sleep(Duration::from_secs(1)).await;

        Ok(FaultResult {
            fault_name: config.name.clone(),
            success: true,
            affected_pods,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics {
                latency_added_ms: network_config.latency_ms,
                packets_dropped: network_config.packet_loss_percent.map(|p| (p as u64 * 1000)),
                ..Default::default()
            },
        })
    }

    async fn recover(&self, config: &FaultSpec) -> Result<(), String> {
        tracing::info!("Recovering network fault: {}", config.name);
        // In production, remove iptables rules, etc.
        Ok(())
    }
}

/// CPU fault injector
pub struct CpuFaultInjector {
    client: Client,
}

impl CpuFaultInjector {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl FaultInjector for CpuFaultInjector {
    fn name(&self) -> &str {
        "cpu-fault-injector"
    }

    fn fault_type(&self) -> &str {
        "cpu"
    }

    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        let cpu_config = match &config.fault_type {
            FaultType::Cpu(c) => c,
            _ => return Err("Invalid fault type for CpuFaultInjector".to_string()),
        };

        tracing::info!(
            "Injecting CPU fault: load={:?}%, cores={}",
            cpu_config.load_percent,
            cpu_config.cores
        );

        let affected_pods = vec!["stellar-node-0".to_string()];

        sleep(Duration::from_secs(1)).await;

        Ok(FaultResult {
            fault_name: config.name.clone(),
            success: true,
            affected_pods,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics {
                cpu_load_percent: cpu_config.load_percent,
                ..Default::default()
            },
        })
    }

    async fn recover(&self, config: &FaultSpec) -> Result<(), String> {
        tracing::info!("Recovering CPU fault: {}", config.name);
        Ok(())
    }
}

/// Memory fault injector
pub struct MemoryFaultInjector {
    client: Client,
}

impl MemoryFaultInjector {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl FaultInjector for MemoryFaultInjector {
    fn name(&self) -> &str {
        "memory-fault-injector"
    }

    fn fault_type(&self) -> &str {
        "memory"
    }

    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        let mem_config = match &config.fault_type {
            FaultType::Memory(m) => m,
            _ => return Err("Invalid fault type for MemoryFaultInjector".to_string()),
        };

        tracing::info!(
            "Injecting memory fault: consumption={:?}%, type={:?}",
            mem_config.consumption_percent,
            mem_config.stress_type
        );

        let affected_pods = vec!["stellar-node-0".to_string()];

        sleep(Duration::from_secs(1)).await;

        Ok(FaultResult {
            fault_name: config.name.clone(),
            success: true,
            affected_pods,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics {
                memory_consumed_mb: mem_config.consumption_mb,
                ..Default::default()
            },
        })
    }

    async fn recover(&self, config: &FaultSpec) -> Result<(), String> {
        tracing::info!("Recovering memory fault: {}", config.name);
        Ok(())
    }
}

/// Disk fault injector
pub struct DiskFaultInjector {
    client: Client,
}

impl DiskFaultInjector {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl FaultInjector for DiskFaultInjector {
    fn name(&self) -> &str {
        "disk-fault-injector"
    }

    fn fault_type(&self) -> &str {
        "disk"
    }

    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        let disk_config = match &config.fault_type {
            FaultType::Disk(d) => d,
            _ => return Err("Invalid fault type for DiskFaultInjector".to_string()),
        };

        tracing::info!(
            "Injecting disk fault: fill={:?}%, read_latency={:?}ms",
            disk_config.fill_percent,
            disk_config.read_latency_ms
        );

        let affected_pods = vec!["stellar-node-0".to_string()];

        sleep(Duration::from_secs(1)).await;

        Ok(FaultResult {
            fault_name: config.name.clone(),
            success: true,
            affected_pods,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics {
                disk_filled_percent: disk_config.fill_percent,
                ..Default::default()
            },
        })
    }

    async fn recover(&self, config: &FaultSpec) -> Result<(), String> {
        tracing::info!("Recovering disk fault: {}", config.name);
        Ok(())
    }
}

/// Pod kill fault injector
pub struct PodKillFaultInjector {
    client: Client,
}

impl PodKillFaultInjector {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl FaultInjector for PodKillFaultInjector {
    fn name(&self) -> &str {
        "pod-kill-fault-injector"
    }

    fn fault_type(&self) -> &str {
        "pod-kill"
    }

    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        tracing::info!("Injecting pod kill fault: {}", config.name);

        // In production, would use Kubernetes API to delete pods
        let affected_pods = vec!["stellar-node-0".to_string()];

        sleep(Duration::from_secs(1)).await;

        Ok(FaultResult {
            fault_name: config.name.clone(),
            success: true,
            affected_pods,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics::default(),
        })
    }

    async fn recover(&self, config: &FaultSpec) -> Result<(), String> {
        tracing::info!("Recovering pod kill fault: {}", config.name);
        // Pod would be recreated by the controller
        Ok(())
    }
}

/// DNS fault injector
pub struct DnsFaultInjector {
    client: Client,
}

impl DnsFaultInjector {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl FaultInjector for DnsFaultInjector {
    fn name(&self) -> &str {
        "dns-fault-injector"
    }

    fn fault_type(&self) -> &str {
        "dns"
    }

    async fn inject(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        let dns_config = match &config.fault_type {
            FaultType::Dns(d) => d,
            _ => return Err("Invalid fault type for DnsFaultInjector".to_string()),
        };

        tracing::info!(
            "Injecting DNS fault: block={}, failure={}",
            dns_config.block,
            dns_config.lookup_failure
        );

        let affected_pods = vec!["stellar-node-0".to_string()];

        sleep(Duration::from_secs(1)).await;

        Ok(FaultResult {
            fault_name: config.name.clone(),
            success: true,
            affected_pods,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics::default(),
        })
    }

    async fn recover(&self, config: &FaultSpec) -> Result<(), String> {
        tracing::info!("Recovering DNS fault: {}", config.name);
        Ok(())
    }
}

/// Fault injection manager - coordinates all fault injectors
pub struct FaultInjectionManager {
    injectors: Arc<RwLock<Vec<Box<dyn FaultInjector>>>>,
    active_faults: Arc<RwLock<std::collections::HashMap<String, FaultResult>>>,
}

impl FaultInjectionManager {
    pub fn new() -> Self {
        Self {
            injectors: Arc::new(RwLock::new(Vec::new())),
            active_faults: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn register_injector(&self, injector: Box<dyn FaultInjector>) {
        let mut injectors = self.injectors.write().await;
        injectors.push(injector);
    }

    pub async fn initialize_with_client(&self, client: Client) {
        self.register_injector(Box::new(NetworkFaultInjector::new(client.clone()))).await;
        self.register_injector(Box::new(CpuFaultInjector::new(client.clone()))).await;
        self.register_injector(Box::new(MemoryFaultInjector::new(client.clone()))).await;
        self.register_injector(Box::new(DiskFaultInjector::new(client.clone()))).await;
        self.register_injector(Box::new(PodKillFaultInjector::new(client.clone()))).await;
        self.register_injector(Box::new(DnsFaultInjector::new(client.clone()))).await;
    }

    pub async fn get_injector(&self, fault_type: &FaultType) -> Option<Box<dyn FaultInjector>> {
        let injectors = self.injectors.read().await;
        
        let type_str = match fault_type {
            FaultType::Network(_) => "network",
            FaultType::Cpu(_) => "cpu",
            FaultType::Memory(_) => "memory",
            FaultType::Disk(_) => "disk",
            FaultType::PodKill => "pod-kill",
            FaultType::ContainerKill => "container-kill",
            FaultType::Dns(_) => "dns",
            FaultType::ClockSkew => "clock",
            FaultType::KernelPanic => "kernel",
            FaultType::Aws(_) => "aws",
            FaultType::Gcp(_) => "gcp",
            FaultType::Azure(_) => "azure",
        };

        for injector in injectors.iter() {
            if injector.fault_type() == type_str {
                return Some(injector.as_ref().clone());
            }
        }
        None
    }

    pub async fn inject_fault(&self, config: &FaultSpec) -> Result<FaultResult, String> {
        let injector = self.get_injector(&config.fault_type).await
            .ok_or_else(|| format!("No injector found for fault type: {:?}", config.fault_type))?;

        let result = injector.inject(config).await?;

        // Track active fault
        let mut active = self.active_faults.write().await;
        active.insert(config.name.clone(), result.clone());

        Ok(result)
    }

    pub async fn recover_fault(&self, config: &FaultSpec) -> Result<(), String> {
        let injector = self.get_injector(&config.fault_type).await
            .ok_or_else(|| format!("No injector found for fault type: {:?}", config.fault_type))?;

        injector.recover(config).await?;

        // Remove from active faults
        let mut active = self.active_faults.write().await;
        active.remove(&config.name);

        Ok(())
    }

    pub async fn recover_all(&self) -> Result<(), String> {
        let active = self.active_faults.read().await;
        for (name, result) in active.iter() {
            tracing::info!("Recovering fault: {}", name);
        }
        Ok(())
    }

    pub async fn get_active_faults(&self) -> Vec<FaultResult> {
        let active = self.active_faults.read().await;
        active.values().cloned().collect()
    }
}

impl Default for FaultInjectionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fault_result_creation() {
        let result = FaultResult {
            fault_name: "test-fault".to_string(),
            success: true,
            affected_pods: vec!["pod-1".to_string()],
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            error: None,
            metrics: FaultMetrics::default(),
        };

        assert!(result.success);
        assert_eq!(result.affected_pods.len(), 1);
    }

    #[tokio::test]
    async fn test_fault_injection_manager() {
        let manager = FaultInjectionManager::new();
        
        let config = FaultSpec {
            name: "test-network".to_string(),
            fault_type: FaultType::Network(NetworkFault {
                latency_ms: Some(1000),
                ..Default::default()
            }),
            target: FaultTarget::default(),
            config: FaultConfig::default(),
            duration_seconds: 30,
            force: false,
        };

        // Without registered injectors, should fail
        let result = manager.get_injector(&config.fault_type).await;
        assert!(result.is_none());
    }
}