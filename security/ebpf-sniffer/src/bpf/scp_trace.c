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
//
// scp_trace.c — eBPF kernel probe for Stellar Consensus Protocol traffic.
//
// Design goals:
//   - Zero added latency: all probes are kprobes/tracepoints; the packet path
//     is never blocked or redirected.  We only observe, never modify.
//   - Kernel verifier safe: every map access is bounds-checked, every pointer
//     is validated before dereference, all loops are bounded, no unbounded
//     stack allocations.
//   - Minimal footprint: one perf-event ring-buffer per CPU for SCP events,
//     one hash map for per-peer state, one hash map for per-peer drop counters.
//
// Probed points:
//   tcp_v4_do_rcv   — intercepts inbound TCP segments on port 11625 (SCP)
//   tcp_v4_send_reset — captures TCP RST packets (cryptographic handshake
//                       failures and firewall-induced drops)
//   tcp_retransmit_skb — tracks retransmit storms per peer
//
// All events are forwarded to the userspace loader via a perf-event array.
// The loader aggregates them into Prometheus metrics and surfaces the data
// through the stellar-operator dashboard REST API.

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <bpf/bpf_tracing.h>
#include <linux/if_ether.h>
#include <linux/ip.h>
#include <linux/tcp.h>
#include <linux/in.h>
#include <linux/socket.h>
#include <net/sock.h>

// ─── Constants ────────────────────────────────────────────────────────────────

/// Well-known SCP peer port (stellar-core default).
#define SCP_PORT 11625

/// Maximum number of simultaneously tracked peers.
#define MAX_PEERS 1024

/// Maximum bytes of XDR payload captured per event (enough for SCP message
/// type discrimination; full payload is NOT copied to avoid latency spikes).
#define XDR_SAMPLE_BYTES 16

/// XDR SCP message type discriminants (big-endian uint32 at offset 0).
/// These correspond to stellar-core's StellarMessage XDR enum variants.
#define XDR_SCP_QUORUM_SET     0x00000002u
#define XDR_SCP_STATEMENT      0x00000003u
#define XDR_SCP_HELLO          0x0000000Eu
#define XDR_SCP_AUTH           0x00000002u

// ─── Event type tags ──────────────────────────────────────────────────────────

#define EVT_SCP_INBOUND        1   // inbound SCP segment received
#define EVT_SCP_TCP_RST        2   // TCP RST sent/received for SCP peer
#define EVT_SCP_RETRANSMIT     3   // TCP retransmit on SCP connection
#define EVT_SCP_HANDSHAKE_FAIL 4   // RST during connection establishment

// ─── Shared structures ────────────────────────────────────────────────────────

/// Kernel-to-userspace event emitted into the perf ring buffer.
/// Kept small (≤ 64 bytes) to minimise per-event allocation overhead.
struct scp_event {
    __u64 timestamp_ns;           // bpf_ktime_get_ns()
    __u32 src_ip;                 // network-byte-order source IP
    __u32 dst_ip;                 // network-byte-order destination IP
    __u16 src_port;               // host-byte-order source port
    __u16 dst_port;               // host-byte-order destination port
    __u8  event_type;             // EVT_SCP_* discriminant
    __u8  tcp_flags;              // TCP flags byte
    __u8  xdr_sample[XDR_SAMPLE_BYTES]; // first bytes of XDR payload
    __u8  xdr_sample_len;         // actual bytes captured (≤ XDR_SAMPLE_BYTES)
    __u8  _pad[5];                // align to 8 bytes; zero-initialised
};

/// Per-peer counters kept in the kernel hash map for fast aggregation.
struct peer_stats {
    __u64 packets_rx;
    __u64 packets_tx;
    __u64 rst_count;
    __u64 retransmit_count;
    __u64 handshake_fail_count;
    __u64 last_seen_ns;
};

/// Key for the peer_stats map: (src_ip, dst_ip, src_port, dst_port) 4-tuple.
/// We normalise direction by putting the SCP side (port 11625) in dst.
struct peer_key {
    __u32 local_ip;
    __u32 remote_ip;
    __u16 local_port;
    __u16 remote_port;
};

// ─── BPF Maps ─────────────────────────────────────────────────────────────────

/// Perf-event ring buffer: kernel pushes scp_event structs; userspace reads them.
struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} scp_events SEC(".maps");

/// Per-peer statistics (hash map, MAX_PEERS entries).
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, MAX_PEERS);
    __type(key, struct peer_key);
    __type(value, struct peer_stats);
} peer_stats_map SEC(".maps");

/// Per-peer TCP RST drop counter exposed to userspace via map iteration.
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, MAX_PEERS);
    __type(key, struct peer_key);
    __type(value, __u64);
} peer_drop_counter SEC(".maps");

// ─── Helper: populate a peer_key from (src, dst, sport, dport) ───────────────

static __always_inline void
make_peer_key(struct peer_key *k,
              __u32 saddr, __u32 daddr,
              __u16 sport, __u16 dport)
{
    // Normalise: local side is whichever end uses port 11625.
    if (dport == SCP_PORT) {
        k->local_ip    = daddr;
        k->remote_ip   = saddr;
        k->local_port  = dport;
        k->remote_port = sport;
    } else {
        k->local_ip    = saddr;
        k->remote_ip   = daddr;
        k->local_port  = sport;
        k->remote_port = dport;
    }
}

// ─── Helper: update peer_stats map entry ─────────────────────────────────────

static __always_inline void
update_peer_stats(struct peer_key *key, __u8 event_type)
{
    struct peer_stats *st = bpf_map_lookup_elem(&peer_stats_map, key);
    if (!st) {
        struct peer_stats zero = {};
        bpf_map_update_elem(&peer_stats_map, key, &zero, BPF_NOEXIST);
        st = bpf_map_lookup_elem(&peer_stats_map, key);
        if (!st)
            return;
    }

    st->last_seen_ns = bpf_ktime_get_ns();

    switch (event_type) {
    case EVT_SCP_INBOUND:
        __sync_fetch_and_add(&st->packets_rx, 1);
        break;
    case EVT_SCP_TCP_RST:
        __sync_fetch_and_add(&st->rst_count, 1);
        break;
    case EVT_SCP_RETRANSMIT:
        __sync_fetch_and_add(&st->retransmit_count, 1);
        break;
    case EVT_SCP_HANDSHAKE_FAIL:
        __sync_fetch_and_add(&st->handshake_fail_count, 1);
        break;
    default:
        break;
    }
}

// ─── Helper: increment drop counter ──────────────────────────────────────────

static __always_inline void
increment_drop_counter(struct peer_key *key)
{
    __u64 *cnt = bpf_map_lookup_elem(&peer_drop_counter, key);
    if (cnt) {
        __sync_fetch_and_add(cnt, 1);
    } else {
        __u64 one = 1;
        bpf_map_update_elem(&peer_drop_counter, key, &one, BPF_NOEXIST);
    }
}

// ─── Helper: emit event to perf ring ─────────────────────────────────────────

static __always_inline void
emit_event(void *ctx, struct scp_event *evt)
{
    // bpf_perf_event_output is non-blocking: if the per-CPU buffer is full,
    // the event is silently dropped (counted separately as lost events in the
    // userspace reader).  This keeps the probe zero-latency for the packet path.
    bpf_perf_event_output(ctx, &scp_events, BPF_F_CURRENT_CPU,
                          evt, sizeof(*evt));
}

// ─── Probe 1: tcp_v4_do_rcv — inbound SCP segment ────────────────────────────
//
// Fired for every inbound TCP segment processed by the IPv4 TCP receive path.
// We filter to port 11625 before touching any packet data to keep the fast path
// overhead negligible for non-SCP traffic.

SEC("kprobe/tcp_v4_do_rcv")
int BPF_KPROBE(probe_tcp_v4_do_rcv, struct sock *sk, struct sk_buff *skb)
{
    if (!sk || !skb)
        return 0;

    // Read local/remote ports from the sock structure.
    __u16 lport = 0, rport = 0;
    bpf_probe_read_kernel(&lport, sizeof(lport),
                          &sk->__sk_common.skc_num);
    bpf_probe_read_kernel(&rport, sizeof(rport),
                          &sk->__sk_common.skc_dport);
    rport = bpf_ntohs(rport);

    // Early exit for non-SCP ports to maintain zero overhead on all other traffic.
    if (lport != SCP_PORT && rport != SCP_PORT)
        return 0;

    __u32 saddr = 0, daddr = 0;
    bpf_probe_read_kernel(&saddr, sizeof(saddr),
                          &sk->__sk_common.skc_rcv_saddr);
    bpf_probe_read_kernel(&daddr, sizeof(daddr),
                          &sk->__sk_common.skc_daddr);

    struct peer_key key = {};
    make_peer_key(&key, saddr, daddr, lport, rport);
    update_peer_stats(&key, EVT_SCP_INBOUND);

    // Build and emit the event.
    struct scp_event evt = {};
    evt.timestamp_ns = bpf_ktime_get_ns();
    evt.src_ip       = saddr;
    evt.dst_ip       = daddr;
    evt.src_port     = lport;
    evt.dst_port     = rport;
    evt.event_type   = EVT_SCP_INBOUND;

    // Attempt to read the first XDR_SAMPLE_BYTES of application payload.
    // The skb->data pointer may be in a non-linear buffer; we use a safe
    // bounded read and accept a short read without failing.
    unsigned char *data_ptr = NULL;
    bpf_probe_read_kernel(&data_ptr, sizeof(data_ptr), &skb->data);
    if (data_ptr) {
        int n = bpf_probe_read_kernel(evt.xdr_sample,
                                      XDR_SAMPLE_BYTES,
                                      data_ptr);
        evt.xdr_sample_len = (n == 0) ? XDR_SAMPLE_BYTES : 0;
    }

    emit_event(ctx, &evt);
    return 0;
}

// ─── Probe 2: tcp_v4_send_reset — TCP RST packets ─────────────────────────────
//
// Fired whenever the kernel sends a TCP RST for an IPv4 connection.  A RST
// on a SCP (port 11625) connection indicates a hard peer rejection — either a
// firewall rule, a TLS handshake failure, or an abrupt peer disconnect.
//
// When we observe a RST during the SYN_SENT or SYN_RECV phase (skb carrying
// SYN+RST or RST before data), we classify it as a handshake failure and
// highlight it in the dashboard as a cryptographic handshake drop.

SEC("kprobe/tcp_v4_send_reset")
int BPF_KPROBE(probe_tcp_v4_send_reset, struct sock *sk, struct sk_buff *skb)
{
    if (!sk)
        return 0;

    __u16 lport = 0, rport = 0;
    bpf_probe_read_kernel(&lport, sizeof(lport),
                          &sk->__sk_common.skc_num);
    bpf_probe_read_kernel(&rport, sizeof(rport),
                          &sk->__sk_common.skc_dport);
    rport = bpf_ntohs(rport);

    if (lport != SCP_PORT && rport != SCP_PORT)
        return 0;

    __u32 saddr = 0, daddr = 0;
    bpf_probe_read_kernel(&saddr, sizeof(saddr),
                          &sk->__sk_common.skc_rcv_saddr);
    bpf_probe_read_kernel(&daddr, sizeof(daddr),
                          &sk->__sk_common.skc_daddr);

    // Determine if this RST occurred during the TCP handshake phase by reading
    // the connection state from the sock.  TCP_SYN_SENT (2) and TCP_SYN_RECV (3)
    // indicate a pre-data handshake abort.
    __u8 sk_state = 0;
    bpf_probe_read_kernel(&sk_state, sizeof(sk_state),
                          &sk->__sk_common.skc_state);

    __u8 evt_type = EVT_SCP_TCP_RST;
    if (sk_state == 2 /* TCP_SYN_SENT */ || sk_state == 3 /* TCP_SYN_RECV */)
        evt_type = EVT_SCP_HANDSHAKE_FAIL;

    // Capture TCP flags if skb is available.
    __u8 tcp_flags = 0;
    if (skb) {
        struct tcphdr *th = NULL;
        bpf_probe_read_kernel(&th, sizeof(th), &skb->data);
        if (th) {
            // tcp_flags is stored in the 13th byte of the TCP header.
            bpf_probe_read_kernel(&tcp_flags, 1,
                                  (unsigned char *)th + 13);
        }
    }

    struct peer_key key = {};
    make_peer_key(&key, saddr, daddr, lport, rport);
    update_peer_stats(&key, evt_type);
    increment_drop_counter(&key);

    struct scp_event evt = {};
    evt.timestamp_ns = bpf_ktime_get_ns();
    evt.src_ip       = saddr;
    evt.dst_ip       = daddr;
    evt.src_port     = lport;
    evt.dst_port     = rport;
    evt.event_type   = evt_type;
    evt.tcp_flags    = tcp_flags;

    emit_event(ctx, &evt);
    return 0;
}

// ─── Probe 3: tcp_retransmit_skb — retransmit tracking ───────────────────────
//
// Fired by the kernel's TCP retransmit timer for both IPv4 and IPv6.  Sustained
// retransmit rates on SCP connections indicate network jitter or peer-link
// degradation and are surfaced in the dashboard latency heatmap.

SEC("kprobe/tcp_retransmit_skb")
int BPF_KPROBE(probe_tcp_retransmit_skb, struct sock *sk, struct sk_buff *skb)
{
    if (!sk)
        return 0;

    __u16 lport = 0, rport = 0;
    bpf_probe_read_kernel(&lport, sizeof(lport),
                          &sk->__sk_common.skc_num);
    bpf_probe_read_kernel(&rport, sizeof(rport),
                          &sk->__sk_common.skc_dport);
    rport = bpf_ntohs(rport);

    if (lport != SCP_PORT && rport != SCP_PORT)
        return 0;

    __u32 saddr = 0, daddr = 0;
    bpf_probe_read_kernel(&saddr, sizeof(saddr),
                          &sk->__sk_common.skc_rcv_saddr);
    bpf_probe_read_kernel(&daddr, sizeof(daddr),
                          &sk->__sk_common.skc_daddr);

    struct peer_key key = {};
    make_peer_key(&key, saddr, daddr, lport, rport);
    update_peer_stats(&key, EVT_SCP_RETRANSMIT);

    struct scp_event evt = {};
    evt.timestamp_ns = bpf_ktime_get_ns();
    evt.src_ip       = saddr;
    evt.dst_ip       = daddr;
    evt.src_port     = lport;
    evt.dst_port     = rport;
    evt.event_type   = EVT_SCP_RETRANSMIT;

    emit_event(ctx, &evt);
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
