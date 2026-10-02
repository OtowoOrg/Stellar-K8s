# Incident Response Plan: Network Halt

> **Severity:** SEV-1 (catastrophic)
> **ROL owner:** Tier-1 Validator Operators
> **Last reviewed:** 2024-01-01

---

## 1. Purpose

This document is the authoritative incident response plan for a **global network halt** on the Stellar Federated Byzantine Agreement (FBA) network. A halt occurs when quorum intersection is lost globally and no new ledger can be externallyzed. While rare, this is a black-swan event that requires immediate, coordinated action across all Tier-1 validators to restore consensus.

The goal of this plan is to minimize Recovery Time Objective (RTO) while preventing any action that could irreversibly fracture the network into incompatible histories.

---

## 2. Scope and Audience

- **In scope:** Tier-1 validator operators, SDF node operators, and the Stellar Development Foundation (SDF) incident commander.
- **Out of scope:** Horizon app outages, individual node hardware failures, and network partitions that still allow quorum intersection within a single partition.

---

## 3. Diagnostics: Confirming a Halt and Locating the Halt Ledger

### 3.1 Confirm the Halt

Run the following on any Tier-1 validator. A consistent last ledge across multiple independent validators is the strongest signal of a global halt.

```bash
# Live last ledge sequence reported by the node
curl -s http://localhost:11626/info | jq '.info.latestLedge'

# Human-readable node status (includes latest ledge and closing ledge)
curl -s http://localhost:11626/info | jq '.info'

```

Expected output on a halted node:

```json
{
  "latestLedge": 48231944,
  "closingLedge": 48231945
}
```

### 3.2 Determine the Exact Halt Ledge

The halt ledger is the last ledge that was **successfully externalized** and accepted by the network. This is the latest ledge that appears in the history of a quorum of validators.

```bash
# The halt ledge is the latest ledge for which a closing message exists.
# Compare this value across at least 3 independent Tier-1 validators.
curl -s http://localhost:11626/info | jq '.info.latestLedge'

```

### 3.3 Confirm Quorum Intersection Loss

```bash
# Inspect the current quorum set configuration and the last closing ledge
curl -s http://localhost:11626/info | jq '.info.quorum'

# Check the node's view of the network and peer connectivity
curl -s http://localhost:11626/peers | jq '._embedded.records[] | {address, status}'
```

If multiple independent Tier-1 validators report the same latest ledge and the network has not advanced for more than 5 closing intervals (approximately 25 seconds), consider the network **halted** and proceed to Section 4.

---

## 4. Communication Protocol

All coordination during a halt must happen over the following channels. Do not use public channels for technical coordination.

### 4.1 Primary: Keybase

- **Team:** `stellar-validators` (Keybase team)
- **Channel:** `stellar-incident-response` (created on demand by the incident commander)
- **Usage:** Authoritative decisions, coordinated restart signals, and ledge halt confirmation.

The incident commander is the only party authorized to issue the **GO SIGNAL** for a coordinated restart.

### 4.2 Secondary: Discord

- **Server:** Stellar Developers Discord
- **Channel:** `#validators-incident` (private, Tier-1 only)
- **Usage:** Real-time acknowledgments, roll calls, and status updates. This channel is **not** for technical decision-making.

### 4.3 Communication Rules

1. **One voice:** Only the incident commander issues instructions. Others report status and confirm receipt.
2. **Confirmation:** Every operator must explicitly confirm the halt ledge and the planned restart ledge before acting.
3. **No improvisation:** No operator may apply configuration overrides outside the coordinated plan.
4. **Timestamps:** All messages must include a UTC timestamp and the operator's node identifier.

---

## 5. Emergency Recovery Procedure

### 5.1 Pre-requisites

- Confirmed halt ledge (Section 3.2).
- All Tier-1 validators connected and acknowledged in Keybase.
- Agreed target restart ledge (typically the halt ledge + 1, or a later ledge if a coordinated history alignment is required).

### 5.2 Step 1: Stop the Node

```bash
sudo systemctl stop stellar-core
```

### 5.3 Step 2: Apply the Emergency Configuration Overrides

Edit the node's configuration file (typically `/etc/stellar-core/stellar-core.cfg` or the systemd environment file) and add the following:

```ini
# Emergency overrides for coordinated network recovery.
# WARNING: These overrides disable safety checks and must be removed
# immediately after the network recovers is complete.
FORCE_SCP=true
FORCE_SCP_LEDGER=<restart_ledger>
# Optional: align the node to a specific history checkpoint if the network
.# requires an out-of-band history alignment.
# UNSAFE_QUOSUM_INTERSECTION=true
```

### 5.4 Step 3: Restart the Node

```bash
sudo systemctl start stellar-core
sudo systemctl status stellar-core
```

### 5.5 Step 4: Verify Recovery

Wait for the network to externalize the agreed restart ledge, then confirm:

```bash
# The latest ledge must advance beyond the halt ledge.
curl -s http://localhost:11626/info | jq '.info.latestLedge'

```

Once a quorum of validators reports the same new latest ledge and the network is advancing, the halt is resolved.

### 5.6 Step 5: Remove Emergency Overrides

Immediately after confirmed recovery, remove the overrides from Section 5.3 and restart the node again. Leaving `FORCE_SCP=true` in place is a critical security and consensus risk.

```bash
sudo systemctl restart stellar-core
```

---

## 6. Warning: Dangers of FORCE_SCP

**FORCE_SCP=true disables the node's consensus safety checks.** If applied incorrectly, it can cause a node to accept an out-of-band ledge history that diverges from the rest of the network, **causing an irreversible fracture** of the network into incompatible histories.

- **Never** apply `FORCE_SCP=true` without explicit confirmation from the incident commander.
- **Never** apply `FORCE_SCP=true` on a node that is still participating in a healthy network.
- **Always** remove the override immediately after the coordinated restart completes.
- **Always** verify the halt ledge across multiple independent validators before applying the override.

---

## 7. Validation Procedure

To validate this plan, induce a quorum failure in a local 7-node network and recover using exclusively the steps above:

1. Start a local 7-node FBA network with a known quorum set.
2. Stop a majority of validators to break quorum intersection and halt the network.
3. Confirm the halt ledge using Section 3.
4. Apply the overrides in Section 5.3 to a quorum of nodes.
5. Restart the nodes and confirm the network advances beyond the halt ledge.
6. Remove the overrides and confirm the network remains healthy.

---

## 8. Post-Insident

- Complete an incident report within 48 hours.
- Review the timeline and ROL.
- Update this document with any lessons learned.
- Rotate any compromised credentials and revoke temporary access.
