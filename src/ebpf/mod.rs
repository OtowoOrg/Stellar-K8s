#[cfg(target_os = "linux")]
use aya::maps::HashMap;
#[cfg(target_os = "linux")]
use aya::programs::Tc;
#[cfg(target_os = "linux")]
use aya::{include_bytes_aligned, Bpf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use stellar_ebpf_common::PacketMetrics;

pub struct EbpfManager {
    #[cfg(target_os = "linux")]
    bpf: Bpf,
}

impl EbpfManager {
    pub fn new() -> Result<Self, anyhow::Error> {
        #[cfg(not(target_os = "linux"))]
        return Err(anyhow::anyhow!("eBPF is only supported on Linux"));

        #[cfg(target_os = "linux")]
        {
            #[cfg(debug_assertions)]
            let data = include_bytes_aligned!("../../target/bpfel-unknown-none/debug/stellar-ebpf");
            #[cfg(not(debug_assertions))]
            let data = include_bytes_aligned!("../../target/bpfel-unknown-none/release/stellar-ebpf");

            let bpf = Bpf::load(data)?;
            Ok(Self { bpf })
        }
    }

    pub fn attach(&mut self, _iface: &str) -> Result<(), anyhow::Error> {
        #[cfg(not(target_os = "linux"))]
        return Err(anyhow::anyhow!("eBPF is only supported on Linux"));

        #[cfg(target_os = "linux")]
        {
            let program: &mut Tc = self.bpf.program_mut("stellar_filter").unwrap().try_into()?;
            program.load()?;
            program.attach(_iface, aya::programs::tc::TcAttachType::Ingress)?;
            info!("Attached eBPF filter to interface {}", _iface);
            Ok(())
        }
    }

    pub fn get_metrics(&self) -> Result<PacketMetrics, anyhow::Error> {
        #[cfg(not(target_os = "linux"))]
        return Ok(PacketMetrics {
            allowed_packets: 0,
            rejected_packets: 0,
            total_bytes: 0,
        });

        #[cfg(target_os = "linux")]
        {
            let metrics_map: HashMap<_, u32, PacketMetrics> = HashMap::try_from(self.bpf.map("METRICS").unwrap())?;
            
            // Key 0 is used for global metrics in our simple eBPF program
            let key = 0u32;
            match metrics_map.get(&key, 0) {
                Ok(m) => Ok(m),
                Err(_) => Ok(PacketMetrics {
                    allowed_packets: 0,
                    rejected_packets: 0,
                    total_bytes: 0,
                }),
            }
        }
    }
}
