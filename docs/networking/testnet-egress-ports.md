# Testnet Peer Egress Port Requirements

> **TL;DR:** Validators need **outbound TCP 3510 and 11625** to reach the SDF testnet peers. Without these egress rules a node boots fine, logs `Joining SCP`, and sits there forever with **zero peers**. Check the [Symptom Checklist](#8-symptom-checklist-zero-peers-joining-scp) before you debug anything else.

This page documents the outbound (egress) port requirements for connecting a Stellar-K8s validator to the SDF testnet. Inbound ports (advertising your own peer port, exposing Horizon) are covered by [troubleshooting/networking.md](../troubleshooting/networking.md) and [bgp-edge-routing.md](bgp-edge-routing.md) — this page is specifically about the **outbound** direction, which is the one most often forgotten in firewalls, NAT gateways, and cloud security groups because "the node doesn't serve anything on it."

Tracking: Closes #1614

---

## 1. Why Outbound Peer Ports Matter

Stellar Core is not a pure server: it is a **dial-out peer**. On startup it opens outbound TCP connections to the peers listed in its `KNOWN_PEERS` / quorum set, and it expects those connections to stay up for flood and SCP messages. Inbound connections alone are not enough — a node that only accepts connections will still fail to join consensus if it cannot dial out to its quorum set.

The SDF-operated testnet validators listen on:

- `3510/tcp` — the peer port used by the SDF testnet validators (testnet default; distinct from the `11625` public-network default).
- `11625/tcp` — the standard Stellar Core peer port, still used by community testnet validators and by `KNOWN_PEERS` entries that run a default configuration.

If either destination is blocked by egress policy, the symptom is always the same: the pod starts, the HTTP admin endpoint answers, the node reports `Joining SCP` — and the connected-peer count stays at zero.

!!! warning "This affects the operator's readiness probe"
    The operator's readiness gate treats `Joining SCP` as **Not Ready** (see [Readiness Probe States](../operations/readiness-probe-states.md)), so a node with blocked egress is also removed from Service endpoints. A stuck validator is an availability problem, not just a cosmetic one.

## 2. Outbound Egress Rules Table

Apply these rules wherever egress is filtered: cloud security groups, NAT gateways, corporate firewalls, and in-cluster `NetworkPolicy`/`CiliumNetworkPolicy` resources.

| # | Direction | Destination | Port | Protocol | Purpose | Notes |
|---|-----------|-------------|------|----------|---------|-------|
| 1 | Egress | SDF testnet validator cores (see §3) | `3510` | TCP | Peer connections to SDF testnet validators | Primary testnet peer port |
| 2 | Egress | SDF testnet validator cores (see §3) | `11625` | TCP | Peer connections to default-port testnet peers | Community/`KNOWN_PEERS` fallback |
| 3 | Egress | History archive (`history.stellar.org`) | `443` | TCP | Ledger history / catchup | Already required; listed for completeness |
| 4 | Egress | Cluster DNS | `53` | UDP + TCP | Resolve peer hostnames | Required by default-deny egress policies |

A minimal default-deny egress policy for a testnet validator therefore looks like:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-testnet-validator-egress
  namespace: stellar-testnet
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/component: stellar-validator
  policyTypes:
    - Egress
  egress:
    # Testnet peer ports — the whole point of this page
    - to:
        - ipBlock:
            cidr: 0.0.0.0/0   # tighten to the SDF testnet ranges below if possible
      ports:
        - port: 3510
          protocol: TCP
        - port: 11625
          protocol: TCP
    - ports:
        - port: 443
          protocol: TCP       # history archives
    - ports:
        - port: 53
          protocol: UDP
        - port: 53
          protocol: TCP       # DNS
```

!!! tip "Prefer name- or CIDR-scoped rules over `0.0.0.0/0` where your CNI supports FQDN filtering (Cilium) or where the SDF publishes peer IP ranges."

## 3. Destinations: Testnet vs Mainnet

The two networks run different defaults, so an egress allow-list that "works for one" silently breaks the other.

| Network | Peer port(s) | Example destinations | Passphrase context |
|---------|--------------|----------------------|--------------------|
| **Testnet** | `3510` and `11625` TCP | SDF testnet cores — the validators named in the [testnet deployment tutorial](../tutorials/deploy-testnet-validator.md) quorum set (`GDKXE2OZ…`, `GCUCJTIY…`, `GC2V2EFS…`) | `Test SDF Network ; September 2015` |
| **Mainnet** | `11625` TCP | Public/mainnet validator cores, discovered via [peer discovery](../peer-discovery.md) | `Public Global Stellar Network ; September 2015` |

Key differences to encode in your firewall:

1. **Port range:** testnet traffic must be allowed to **both** `3510` and `11625`; mainnet only needs `11625`. A ruleset copied from a mainnet validator that omits `3510` is the most common cause of a testnet node stuck at zero peers.
2. **Destinations must match the network.** A testnet node must *not* be able to reach mainnet peers, and vice versa. Cross-network connections corrupt consensus and are blocked by the operator's isolation policies (see [network-isolation.md](../network-isolation.md) and the `stellar.org/network` namespace labels in the Helm chart). Your egress rules are the outer layer of that guarantee: scope destination IPs/FQDNs to the network the namespace is labelled for.
3. **Do not share one allow-list across namespaces.** Keep per-network egress policies separate so an audit can see at a glance which destinations a testnet namespace may dial.

## 4. Firewall Guidance

At a host firewall (`iptables`/`nftables`/`ufw`) or perimeter firewall:

- **Stateful allow is enough for egress:** allow NEW + ESTABLISHED outbound TCP to destinations on ports `3510` and `11625`. Return traffic is accepted by the established-state rule — you do **not** need matching inbound rules for the dial-out direction (you still need inbound `11625` if *other* nodes dial *you*, see §6).
- **Log the drops first.** Before opening anything, add a `LOG` rule for outbound `--dport 3510` / `--dport 11625` rejections; this converts "stuck at Joining SCP" into visible evidence within seconds.
- **Order matters:** a REJECT/DROP rule earlier in the chain wins. Insert the allow rules *above* any default-deny egress rule.
- **Egress filtering appliances** (transparent proxies, TLS-inspection gateways) must pass peer traffic through untouched — Stellar P2P is a raw binary protocol; do not route it through an HTTP proxy.

```bash
# Example: nftables allow-list for a testnet validator node
nft add rule ip filter OUTPUT ip daddr <sdf-testnet-cidr> tcp dport { 3510, 11625 } ct state new,established accept
nft add rule ip filter OUTPUT tcp dport { 3510, 11625 } ct state invalid,related counter log prefix "stellar-egress-drop " drop
```

## 5. NAT Guidance

Validators commonly run behind NAT. Dial-out through NAT works with standard stateful NAT, but watch for these traps:

- **SNAT/masquerade is required** for pod traffic leaving the cluster toward testnet cores. If pods use RFC1918 addresses routed to a NAT gateway, confirm the gateway SNATs them — otherwise the remote peer sees an unroutable source and resets the connection.
- **No static port mapping is needed for dial-out.** Unlike inbound peering, you do not need a 1:1 DNAT or port forward for outbound `3510`/`11625`; the connection is tracked by conntrack.
- **Conntrack/UDP-style timeouts don't apply, but idle TCP reaps do.** Ensure the NAT device's established-TCP idle timeout is comfortably above Stellar's keepalive behavior (≥ 30 minutes is a safe baseline); aggressive timeouts manifest as peers repeatedly reconnecting and ledger closes stuttering.
- **Egress gateway / dedicated NAT IP:** if your cloud credits a static egress IP, allow-list *that IP* on the destination side only if the destination itself filters sources — the SDF testnet cores generally do not, so this is optional.
- **Hairpin NAT** (pod → NAT → same cluster's own LoadBalancer IP) is a common source of confusion in test rigs: peers configured with your own external address will hairpin; prefer internal Service DNS names for intra-cluster peers.

## 6. Security Group Guidance (AWS / GCP / Azure)

Security groups are where the `3510` omission bites hardest, because cloud consoles default to "allow outbound 0.0.0.0/0" — and hardened clusters immediately remove that.

### AWS

- **Worker-node security group (egress rules):**

| Type | Protocol | Port range | Destination | Purpose |
|------|----------|------------|-------------|---------|
| Custom TCP | TCP | `3510` | SDF testnet core CIDRs / `0.0.0.0/0` (scoped) | Peer dial-out to SDF testnet |
| Custom TCP | TCP | `11625` | SDF testnet core CIDRs / `0.0.0.0/0` (scoped) | Peer dial-out (default-port peers) |
| HTTPS | TCP | `443` | `0.0.0.0/0` | History archives |

- The NetworkPolicy in §2 only filters at pod level; the **instance-level SG must also allow the same ports**, and both layers must agree. Debug order: pod policy → node SG → VPC firewall/NACL → upstream corporate firewall.
- If you use a **NAT gateway**, its SG allows all outbound by default; verify nobody "hardened" it by copying the instance SG.

### GCP

- VPC firewall **egress rules** are deny-by-default only if a higher-priority deny exists. Add an egress allow rule with `--rules=tcp:3510,tcp:11625` and destination ranges covering the testnet cores.
- If **Cloud NAT** is used, confirm NAT config covers the node subnet and that no higher-priority egress deny rule shadows it.

### Azure

- Azure default outbound access is being retired; if you route through a **Azure Firewall** or NVA, add an application/network rule for `TCP 3510` and `TCP 11625` to the testnet destinations.
- **NSG** egress rules: add `AllowTcpOut3510` and `AllowTcpOut11625` rules at a priority *lower number* (higher precedence) than any default-deny egress rule.

!!! note "LoadBalancer inbound is a different surface"
    Opening inbound `11625` on a LoadBalancer/SG so others can dial you does **not** satisfy dial-out — both directions are required for reliable peering (§1).

## 7. Verification: Confirm Egress Works

Run these **before** deploying a validator, from a debug pod in the validator's namespace:

```bash
# 1. TCP reachability to a known SDF testnet peer (port 3510)
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -n stellar-testnet -- \
  nc -zv -w 5 <sdf-testnet-host-1> 3510

# 2. Same for the default peer port (11625)
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -n stellar-testnet -- \
  nc -zv -w 5 <sdf-testnet-host-2> 11625

# 3. Live peer count once the validator is up (11626 = core HTTP admin port)
kubectl exec -n stellar-testnet <validator-pod> -- \
  curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'

# 4. Watch egress drops at the CNI level (Cilium example)
kubectl exec -n kube-system <cilium-pod> -- \
  cilium monitor --type drop | grep -E '3510|11625'
```

Expected results:

| Check | Healthy | Egress blocked |
|-------|---------|----------------|
| `nc` to peer port | `succeeded!` | times out / `No route to host` |
| `curl /peers` count | ≥ 1 within minutes of start | `0` indefinitely |
| `cilium monitor` / SG flow logs | allows | repeated `denied` / `REJECT` entries |

## 8. Symptom Checklist: Zero Peers / `Joining SCP`

Map the kubectl-visible state to the cause. If rows 1–3 are all true, missing egress to `3510`/`11625` is the leading suspect.

| Observable (kubectl / core admin) | What you see | Most likely cause |
|-----------------------------------|--------------|-------------------|
| Pod phase | `Running`, restarts stable | Not a crash — connectivity issue |
| `curl localhost:11626/info` → `.state` | `Joining SCP` (never reaches `Synced!`) | Cannot reach quorum — egress blocked (this page) or quorum misconfig |
| Peer count (`/peers`) | `0` indefinitely | Outbound `3510`/`11625` denied by SG/NAT/NetworkPolicy |
| Core logs | Repeated `Trying to connect to <peer>` / connect timeouts | Same as above — cross-check drop logs from §4 |
| Readiness | Pod `0/1` (NotReady) per the [readiness state table](../operations/readiness-probe-states.md) | Downstream symptom of the same root cause |

Escalation path when symptoms match but ports verify as open:

1. Wrong `NETWORK_PASSPHRASE` (testnet vs mainnet mismatch — see §3).
2. Quorum set does not include reachable validators ([testnet compliance checks](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/testnet_compliance.rs) surface this at reconcile time).
3. Inbound direction blocked on the *remote* side, or your node's own inbound `11625` is blocked so peers can't dial back (see §6 note).
4. DNS resolution failing inside the pod ([DNS section of the troubleshooting guide](../troubleshooting/networking.md#7-dns-resolution-issues)).

## 9. Related Documentation

- [Networking Troubleshooting Guide](../troubleshooting/networking.md) — full port map, ingress/LB diagnosis
- [Peer Discovery](../peer-discovery.md) — how peers are resolved into dial targets
- [Network Isolation](../network-isolation.md) — testnet/mainnet segregation guarantees
- [Readiness Probe States](../operations/readiness-probe-states.md) — the state machine behind `NotReady`
- [Deploy a Testnet Validator (tutorial)](../tutorials/deploy-testnet-validator.md) — end-to-end testnet deployment
