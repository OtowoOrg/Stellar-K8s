// SPDX-License-Identifier: Apache-2.0
// Copyright 2024 Stellar-K8s Contributors
//
// xdp_scp_kern.c — Kernel-side eBPF XDP program skeleton.
//
// COMPILATION (not done by `cargo build`):
//   clang -O2 -g -target bpf -D__TARGET_ARCH_x86 \
//         -I/usr/include/bpf                       \
//         -c xdp_scp_kern.c -o xdp_scp_kern.o
//   bpftool gen skeleton xdp_scp_kern.o > xdp_scp_kern.skel.h
//
// REQUIREMENTS: kernel >= 5.8, BTF enabled, CAP_BPF + CAP_NET_ADMIN.
//
// This file is a documentation/integration stub.  The userspace simulation
// in ebpf/mod.rs provides identical semantics without kernel dependencies.

#include <linux/bpf.h>
#include <linux/if_ether.h>
#include <linux/ip.h>
#include <linux/tcp.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>

#define SCP_PORT   11625
#define SAMPLE_LEN 256

/* Ring-buffer map: kernel → userspace packet events */
struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 1 << 24); /* 16 MiB */
} packet_events SEC(".maps");

/* Per-IP packet counter map for kernel-side flood pre-filter */
struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 65536);
    __type(key,   __u32);  /* source IPv4 */
    __type(value, __u64);  /* packet count */
} ip_counters SEC(".maps");

/* Drop-list map: populated by userspace blacklist manager */
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 4096);
    __type(key,   __u32);  /* source IPv4 */
    __type(value, __u8);   /* 1 = drop */
} blacklist_map SEC(".maps");

struct pkt_event {
    __u32 src_ip;
    __u16 src_port;
    __u16 dst_port;
    __u64 ts_ns;
    __u32 payload_len;
    __u8  payload[SAMPLE_LEN];
};

SEC("xdp")
int xdp_scp_filter(struct xdp_md *ctx) {
    void *data     = (void *)(long)ctx->data;
    void *data_end = (void *)(long)ctx->data_end;

    /* Parse Ethernet header */
    struct ethhdr *eth = data;
    if ((void *)(eth + 1) > data_end)
        return XDP_PASS;
    if (eth->h_proto != bpf_htons(ETH_P_IP))
        return XDP_PASS;

    /* Parse IP header */
    struct iphdr *ip = (void *)(eth + 1);
    if ((void *)(ip + 1) > data_end)
        return XDP_PASS;
    if (ip->protocol != IPPROTO_TCP)
        return XDP_PASS;

    __u32 src_ip = ip->saddr;

    /* Check blacklist — drop before further processing */
    __u8 *blocked = bpf_map_lookup_elem(&blacklist_map, &src_ip);
    if (blocked && *blocked)
        return XDP_DROP;

    /* Parse TCP header */
    struct tcphdr *tcp = (void *)ip + (ip->ihl * 4);
    if ((void *)(tcp + 1) > data_end)
        return XDP_PASS;
    if (bpf_ntohs(tcp->dest) != SCP_PORT)
        return XDP_PASS;

    /* Increment per-IP counter */
    __u64 one = 1;
    __u64 *cnt = bpf_map_lookup_elem(&ip_counters, &src_ip);
    if (cnt) {
        __sync_fetch_and_add(cnt, 1);
    } else {
        bpf_map_update_elem(&ip_counters, &src_ip, &one, BPF_NOEXIST);
    }

    /* Emit packet event to ring-buffer */
    struct pkt_event *ev = bpf_ringbuf_reserve(&packet_events, sizeof(*ev), 0);
    if (!ev)
        return XDP_PASS;

    ev->src_ip    = src_ip;
    ev->src_port  = bpf_ntohs(tcp->source);
    ev->dst_port  = SCP_PORT;
    ev->ts_ns     = bpf_ktime_get_ns();

    /* Safely copy payload bytes */
    void *payload_start = (void *)tcp + (tcp->doff * 4);
    __u32 payload_len   = (void *)data_end - payload_start;
    if (payload_len > SAMPLE_LEN)
        payload_len = SAMPLE_LEN;
    ev->payload_len = payload_len;

    if (payload_len > 0 && payload_start + payload_len <= data_end)
        __builtin_memcpy(ev->payload, payload_start, payload_len);

    bpf_ringbuf_submit(ev, 0);
    return XDP_PASS;
}

char _license[] SEC("license") = "Apache-2.0";
