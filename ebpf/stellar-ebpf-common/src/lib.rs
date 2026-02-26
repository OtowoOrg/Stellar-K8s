#![no_std]

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PacketMetrics {
    pub allowed_packets: u64,
    pub rejected_packets: u64,
    pub total_bytes: u64,
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for PacketMetrics {}
