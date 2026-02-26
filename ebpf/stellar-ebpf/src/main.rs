#![no_std]
#![no_main]

use aya_ebpf::{
    macros::{classifier, map},
    maps::HashMap,
    programs::TcContext,
};
use core::mem;
use network_types::{
    eth::{EthHdr, EtherType},
    ip::Ipv4Hdr,
    tcp::TcpHdr,
};
use stellar_ebpf_common::PacketMetrics;

#[map]
static METRICS: HashMap<u32, PacketMetrics> = HashMap::<u32, PacketMetrics>::with_max_entries(1024, 0);

#[classifier]
pub fn stellar_filter(ctx: TcContext) -> i32 {
    match try_stellar_filter(ctx) {
        Ok(ret) => ret,
        Err(_) => 0, // TC_ACT_OK
    }
}

#[inline(always)]
fn ptr_at<T>(ctx: &TcContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = mem::size_of::<T>();

    if start + offset + len > end {
        return Err(());
    }

    Ok((start + offset) as *const T)
}

fn try_stellar_filter(ctx: TcContext) -> Result<i32, ()> {
    let ethhdr: *const EthHdr = ptr_at(&ctx, 0)?;
    if unsafe { (*ethhdr).ether_type } != EtherType::Ipv4 {
        return Ok(0);
    }

    let ipv4hdr: *const Ipv4Hdr = ptr_at(&ctx, EthHdr::LEN)?;
    if unsafe { (*ipv4hdr).proto } != network_types::ip::IpProto::Tcp {
        return Ok(0);
    }

    let tcphdr: *const TcpHdr = ptr_at(&ctx, EthHdr::LEN + Ipv4Hdr::LEN)?;
    let dest_port = u16::from_be(unsafe { (*tcphdr).dest });

    if dest_port != 11625 {
        return Ok(0);
    }

    let tcp_offset = (unsafe { (*tcphdr).doff() } * 4) as usize;
    let payload_offset = EthHdr::LEN + Ipv4Hdr::LEN + tcp_offset;
    
    let data_end = ctx.data_end();
    let data_start = ctx.data();
    let total_len = data_end - data_start;

    if data_start + payload_offset + 4 <= data_end {
        let record_len_ptr = (data_start + payload_offset) as *const u32;
        let record_len = u32::from_be(unsafe { *record_len_ptr });

        // Stellar record length sanity check (max 16MB)
        if record_len > 16 * 1024 * 1024 {
            update_metrics(false, total_len as u64);
            return Ok(2); // TC_ACT_SHOT
        }

        if data_start + payload_offset + 8 <= data_end {
            let version_ptr = (data_start + payload_offset + 4) as *const u32;
            let version = u32::from_be(unsafe { *version_ptr });

            // Stellar AuthenticatedMessage version must be 0
            if version != 0 {
                update_metrics(false, total_len as u64);
                return Ok(2); // TC_ACT_SHOT
            }
        }
    }

    update_metrics(true, total_len as u64);
    Ok(0) // TC_ACT_OK
}

fn update_metrics(allowed: bool, bytes: u64) {
    let key = 0u32;
    if let Some(metrics) = METRICS.get_ptr_mut(&key) {
        unsafe {
            (*metrics).total_bytes += bytes;
            if allowed {
                (*metrics).allowed_packets += 1;
            } else {
                (*metrics).rejected_packets += 1;
            }
        }
    } else {
        let metrics = PacketMetrics {
            allowed_packets: if allowed { 1 } else { 0 },
            rejected_packets: if allowed { 0 } else { 1 },
            total_bytes: bytes,
        };
        let _ = METRICS.insert(&key, &metrics, 0);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
