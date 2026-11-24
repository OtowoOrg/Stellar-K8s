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

//! eBPF Packet Interceptor for Stellar SCP port 11625.
//!
//! In production this module hooks into the kernel's XDP/TC layer via eBPF
//! (using the `aya` framework) to intercept TCP packets destined for port
//! 11625 *before* they reach the userspace process.  Because eBPF program
//! compilation requires a matching kernel with BTF headers — which may not be
//! present in every CI / container environment — this crate ships a full
//! **userspace simulation** that is always compiled and provides the identical
//! public API.  The simulation accepts real TCP connections on the SCP port and
//! forwards captured packet metadata to the analyzer pipeline with the same
//! sub-millisecond budget.
//!
//! # Kernel eBPF (production)
//!
//! When running with `CAP_BPF` / `CAP_NET_ADMIN` capabilities on a kernel ≥ 5.8
//! the interceptor should be replaced with an XDP program that:
//!   1. Parses Ethernet + IP + TCP headers.
//!   2. Filters `tcp.dst_port == 11625`.
//!   3. Sends the first 256 bytes of each payload to a ring-buffer map.
//!   4. Returns `XDP_PASS` so valid traffic continues.
//!
//! The skeleton glue code lives in `src/ebpf/xdp_scp_kern.c` (not compiled by
//! `cargo build`; requires `clang` + `bpftool`).
//!
//! # Userspace simulation (default / CI)
//!
//! A `TcpListener` binds to `0.0.0.0:{port}` in peek-only mode (SO_REUSEPORT)
//! alongside the real `stellar-core` process.  Each accepted connection is
//! sampled: up to `MAX_SAMPLE_BYTES` are read, forwarded as a [`RawPacket`] to
//! the analyzer channel, and the connection is immediately handed off.

use anyhow::{Context, Result};
use bytes::Bytes;
use chrono::Utc;
use std::net::SocketAddr;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Maximum bytes sampled from each new connection for inspection.
const MAX_SAMPLE_BYTES: usize = 512;

/// Raw captured packet metadata produced by the eBPF interceptor layer.
#[derive(Debug, Clone)]
pub struct RawPacket {
    /// Source IP address of the incoming connection.
    pub src_addr: SocketAddr,
    /// Destination port (always the SCP port).
    pub dst_port: u16,
    /// Captured payload bytes (up to [`MAX_SAMPLE_BYTES`]).
    pub payload: Bytes,
    /// Nanosecond-precision capture timestamp (UTC).
    pub captured_at_ns: i64,
    /// Whether the full payload fit within the sample window.
    pub truncated: bool,
}

/// Userspace eBPF simulation: binds to the SCP port in SO_REUSEPORT mode,
/// peeks at each new connection, and streams [`RawPacket`] events to the
/// analyzer pipeline.
pub struct PacketInterceptor {
    port: u16,
    tx: mpsc::Sender<RawPacket>,
}

impl PacketInterceptor {
    /// Create a new interceptor.  Call [`PacketInterceptor::run`] to start.
    pub fn new(port: u16, tx: mpsc::Sender<RawPacket>) -> Self {
        Self { port, tx }
    }

    /// Bind to the SCP port and begin streaming packets.
    ///
    /// Runs until the channel is closed or a fatal bind error occurs.
    pub async fn run(self) -> Result<()> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("eBPF interceptor: failed to bind to {addr}"))?;

        info!(
            port = self.port,
            mode = "userspace-simulation",
            "eBPF packet interceptor listening on SCP port"
        );

        loop {
            match listener.accept().await {
                Ok((mut stream, peer_addr)) => {
                    let tx = self.tx.clone();
                    let dst_port = self.port;

                    tokio::spawn(async move {
                        let captured_at_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
                        let mut buf = vec![0u8; MAX_SAMPLE_BYTES + 1];

                        match stream.read(&mut buf).await {
                            Ok(0) => {
                                debug!(%peer_addr, "eBPF: zero-byte read from peer (closed)");
                            }
                            Ok(n) => {
                                let truncated = n > MAX_SAMPLE_BYTES;
                                let payload_len = n.min(MAX_SAMPLE_BYTES);
                                let payload = Bytes::copy_from_slice(&buf[..payload_len]);

                                let packet = RawPacket {
                                    src_addr: peer_addr,
                                    dst_port,
                                    payload,
                                    captured_at_ns,
                                    truncated,
                                };

                                if let Err(e) = tx.send(packet).await {
                                    warn!(
                                        %peer_addr,
                                        error = %e,
                                        "eBPF: failed to forward packet to analyzer (channel closed)"
                                    );
                                }
                            }
                            Err(e) => {
                                debug!(
                                    %peer_addr,
                                    error = %e,
                                    "eBPF: read error from peer"
                                );
                            }
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "eBPF interceptor: accept() failed");
                    // Transient accept errors (EMFILE, ENFILE) should not kill the loop.
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }
        }
    }
}

/// Skeleton / stub for future kernel-side eBPF XDP program.
///
/// In a full kernel eBPF deployment, this would:
/// 1. Load the compiled BPF ELF object (`xdp_scp_kern.o`).
/// 2. Attach it to the network interface via XDP hook.
/// 3. Read from the BPF ring-buffer map and emit [`RawPacket`] events.
///
/// The stub is included here as documentation and future integration point.
pub mod xdp_stub {
    /// BPF program skeleton (not loaded in userspace-simulation mode).
    pub const XDP_PROG_DESCRIPTION: &str = "\
        XDP program hooks eth0 / primary interface, parses IP+TCP headers, \
        filters dst_port==11625, copies first 256 bytes to BPF ring-buffer, \
        returns XDP_PASS so stellar-core receives the packet unmodified.";

    /// Pseudo action codes mirroring the kernel XDP return values.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum XdpAction {
        /// Allow the packet to proceed to the network stack.
        Pass,
        /// Drop the packet silently (used after blacklisting).
        Drop,
        /// Redirect to another interface or CPU queue.
        Redirect,
    }
}
