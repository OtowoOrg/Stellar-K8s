# Testnet Peer Configuration Reference

This document specifies the exact peer ports, network passphrase, and quorum set configuration required to connect a Stellar-K8s validator to the SDF testnet.

## Quick Reference

| Setting | Testnet Value | Mainnet Value (for comparison) |
|---------|---------------|-------------------------------|
| **Network Passphrase** | `Test SDF Network ; September 2015` | `Public Global Stellar Network ; September 2015` |
| **Peer Port 1** | `3510` (TCP, SDF default) | `11625` (TCP only) |
| **Peer Port 2** | `11625` (TCP, fallback/KNOWN_PEERS) | N/A |
| **Horizon Port** | `11626` (HTTP admin API) | `11626` (HTTP admin API) |
| **History Archive** | `https://history.stellar.org/prd/core-testnet/` | `https://history.stellar.org/prd/core-live/` |

!!! note "Dual-port requirement"
    Testnet validators must be able to reach **both** `3510` and `11625` TCP. Omitting `3510` is the most common cause of a testnet validator stuck at `Joining SCP` with zero peers.

---

## Network Passphrase

The network passphrase is a unique identifier that separates testnet from mainnet at the protocol level. All transactions, quorum sets, and consensus messages include the passphrase; a mismatch causes the node to isolate itself from the network.

**Testnet passphrase:**
```
Test SDF Network ; September 2015
```

**Where to configure it:**

In your `StellarNode` manifest:
```yaml
spec:
  network: testnet  # This automatically sets the correct passphrase
  # Alternatively, if using custom network config:
  # config:
  #   networkPassphrase: "Test SDF Network ; September 2015"
```

In stellar-core config (`stellar-core.cfg`):
```ini
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
```

---

## Peer Ports and Egress Rules

Testnet validators connect to peers on two ports. Both must be reachable.

### Port 3510 (SDF Testnet Default)

- **Used by:** SDF-operated testnet validator cores
- **Protocol:** TCP
- **Purpose:** P2P peer connections (SCP, transaction propagation)
- **Required?** Yes — without this port, you cannot reach SDF testnet cores

**Egress firewall rule (Linux iptables):**
```bash
iptables -A OUTPUT -p tcp --dport 3510 -j ACCEPT
```

**Egress firewall rule (nftables):**
```bash
nft add rule ip filter OUTPUT tcp dport 3510 ct state new,established accept
```

**AWS Security Group (outbound rule):**
| Type | Protocol | Port Range | Destination | Purpose |
|------|----------|-----------|-------------|---------|
| Custom TCP | TCP | 3510 | 0.0.0.0/0 (or scoped to SDF CIDR) | Testnet peer dial-out |

**Azure NSG (outbound rule):**
```
Name: AllowTcpOut3510
Direction: Outbound
Priority: 100 (lower = higher priority)
Protocol: TCP
Port Range: 3510
Destination: * (or scoped to SDF CIDR)
Action: Allow
```

**GCP VPC Firewall (egress rule):**
```bash
gcloud compute firewall-rules create allow-stellar-testnet-3510 \
  --direction=EGRESS \
  --priority=1000 \
  --destination-ranges=0.0.0.0/0 \
  --allow=tcp:3510
```

### Port 11625 (Standard Stellar Peer Port)

- **Used by:** Community testnet validators, fallback for `KNOWN_PEERS`
- **Protocol:** TCP
- **Purpose:** P2P peer connections (same as above)
- **Required?** Yes — some testnet peers use this port

**Egress firewall rule (Linux iptables):**
```bash
iptables -A OUTPUT -p tcp --dport 11625 -j ACCEPT
```

**Egress firewall rule (nftables):**
```bash
nft add rule ip filter OUTPUT tcp dport 11625 ct state new,established accept
```

**AWS Security Group (outbound rule):**
| Type | Protocol | Port Range | Destination | Purpose |
|------|----------|-----------|-------------|---------|
| Custom TCP | TCP | 11625 | 0.0.0.0/0 (or scoped to SDF CIDR) | Testnet peer fallback |

---

## Quorum Set Configuration

The quorum set defines which validators your node trusts for consensus. For testnet, use the SDF testnet validators as your root quorum.

### SDF Testnet Validator Public Keys

These are the canonical SDF-operated testnet validator keys. Use them in your quorum set.

```yaml
GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y  # SDF testnet 1
GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP  # SDF testnet 2
GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z  # SDF testnet 3
GBBD3OI4HVJ5VLCVESMMKYTFLYMOMTQBMPOQKRLQYY5RJLexcbv7D5V  # SDF testnet 4
GBJBVKNQ4HCFX5JBFXNM5LNMUYVN5VXWKF3HEJ3K5X2XRLL3VDE...  # (additional testnet validators)
```

### Manifest Configuration

In your `StellarNode` manifest:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: testnet-validator
  namespace: stellar-testnet
spec:
  nodeType: Validator
  network: testnet
  
  validatorConfig:
    # Your validator's keys
    seedSecretRef: "validator-seed"  # K8s secret containing your seed
    
    # Quorum set configuration
    quorumSet:
      threshold: 2  # Require 2-of-3 SDF nodes to agree
      validators:
        - "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y"
        - "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP"
        - "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z"
        - "$self"  # Include yourself in the quorum
```

### stellar-core.cfg Configuration

If you're using a custom `stellar-core.cfg`:

```ini
[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=[
  "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y@testnet-core1.stellar.org",
  "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP@testnet-core2.stellar.org",
  "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z@testnet-core3.stellar.org",
  "$self"
]
```

---

## Known Peers (KNOWN_PEERS)

`KNOWN_PEERS` is a fallback list of peers to try if the quorum set is unreachable. For testnet, include the SDF cores.

### Manifest Configuration

```yaml
spec:
  validatorConfig:
    knownPeers:
      - "testnet-core1.stellar.org"
      - "testnet-core2.stellar.org"
      - "testnet-core3.stellar.org"
```

### stellar-core.cfg Configuration

```ini
[KNOWN_PEERS]
PEERS=[
  "testnet-core1.stellar.org",
  "testnet-core2.stellar.org",
  "testnet-core3.stellar.org"
]
```

---

## History Archive Configuration

Testnet uses a separate history archive. Configure your validator to download ledger history from the testnet archive.

### Manifest Configuration

```yaml
spec:
  validatorConfig:
    historyArchives:
      get:
        - "https://history.stellar.org/prd/core-testnet/"
      put:
        - "https://history.stellar.org/prd/core-testnet/"  # Only if archiving
```

### stellar-core.cfg Configuration

```ini
[HISTORY]
# Get history from the testnet archive
get=https://history.stellar.org/prd/core-testnet/

# Publish history (requires credentials; omit if read-only)
put=https://history.stellar.org/prd/core-testnet/
```

---

## Complete Example Manifest

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-testnet-validator
  namespace: stellar-testnet
spec:
  # Network & node type
  nodeType: Validator
  network: testnet
  version: "v21.0.0"

  # Validator configuration
  validatorConfig:
    # Your validator keys (stored in K8s secret)
    seedSecretRef: "validator-seed"
    
    # Public network visibility
    publicNetwork: true
    
    # Enable validator mode
    nodeIsValidator: true
    
    # Quorum set (must include SDF testnet cores)
    quorumSet:
      threshold: 2
      validators:
        - "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y"
        - "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP"
        - "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z"
        - "$self"
    
    # Known peers (fallback)
    knownPeers:
      - "testnet-core1.stellar.org"
      - "testnet-core2.stellar.org"
      - "testnet-core3.stellar.org"
    
    # History archive
    historyArchives:
      get:
        - "https://history.stellar.org/prd/core-testnet/"
    
    # Catchup configuration
    catchupRecent: 1024  # Catch up last 1024 ledgers on startup

  # Storage
  storage:
    size: 100Gi
    storageClass: "standard"
    retentionPolicy: Retain

  # Resources
  resources:
    requests:
      cpu: "2"
      memory: "4Gi"
    limits:
      cpu: "4"
      memory: "8Gi"

  # Monitoring
  monitoring:
    enabled: true
```

---

## Verification Checklist

Before deploying, verify these settings:

- [ ] **Network passphrase** set to `Test SDF Network ; September 2015`
- [ ] **Peer ports 3510 and 11625** are open outbound to testnet cores
- [ ] **Quorum set** includes at least the three SDF testnet validators
- [ ] **History archive** points to `https://history.stellar.org/prd/core-testnet/`
- [ ] **Namespace** is labelled for testnet isolation (e.g., `stellar.org/network: testnet`)
- [ ] **Known peers** includes at least one SDF testnet core

---

## Debugging Peer Connectivity

If your validator is not connecting to peers:

### 1. Verify egress ports are open

From a debug pod in the validator's namespace:

```bash
# Test port 3510
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -n stellar-testnet -- \
  nc -zv -w 5 testnet-core1.stellar.org 3510

# Test port 11625
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -n stellar-testnet -- \
  nc -zv -w 5 testnet-core2.stellar.org 11625
```

Expected output: `succeeded!`

### 2. Check peer count in the running validator

```bash
kubectl exec -n stellar-testnet <pod> -- curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'
```

Expected: ≥ 1 within 2–5 minutes of pod start

### 3. Check core logs for connection attempts

```bash
kubectl logs -n stellar-testnet <pod> -f | grep -i "connect\|peer"
```

Expected: Log entries showing connection attempts and successful peer connections

### 4. Verify DNS resolution

```bash
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -n stellar-testnet -- \
  nslookup testnet-core1.stellar.org
```

Expected: Resolves to one or more IPs

---

## Related Documentation

- [Testnet Peer Egress Port Requirements](../networking/testnet-egress-ports.md) — detailed firewall guidance
- [Deploy a Testnet Validator](../tutorials/deploy-testnet-validator.md) — step-by-step tutorial
- [API Reference](../api-reference.md) — full CRD field documentation
- [Configuration Guide](index.md) — all configuration options

