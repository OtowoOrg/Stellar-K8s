# MetalLB/BGP Anycast for Global Node Discovery

This guide explains how to configure MetalLB with BGP Anycast for global Stellar node discovery. This enables geographic load distribution, automatic failover, and improved latency for users connecting to your Stellar infrastructure.

## Overview

BGP Anycast allows multiple Stellar nodes across different geographic regions to share the same IP address. When clients connect to this IP, they are automatically routed to the nearest healthy node based on BGP routing metrics.

### Benefits

- **Geographic Load Distribution**: Traffic is automatically routed to the nearest node
- **Automatic Failover**: If a node fails, traffic is rerouted to the next nearest healthy node
- **Improved Latency**: Users connect to the closest node, reducing round-trip time
- **Seamless Scaling**: Add or remove nodes without changing client configurations

## Prerequisites

1. **MetalLB** installed in your Kubernetes cluster
2. **BGP-capable routers** in your network infrastructure
3. **Coordinated ASN and IP addresses** with your network team
4. **Firewall rules** allowing BGP traffic (TCP port 179)

### Install MetalLB

```bash
helm repo add metallb https://metallb.github.io/metallb
helm install metallb metallb/metallb -n metallb-system --create-namespace
```

## Configuration

### Basic LoadBalancer (L2 Mode)

For simple local network deployments, use L2 mode:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-l2
spec:
  nodeType: Horizon
  network: Mainnet
  version: "v21.0.0"
  
  loadBalancer:
    enabled: true
    mode: L2
    addressPool: "stellar-pool"
    loadBalancerIP: "192.168.1.100"
    externalTrafficPolicy: Local
```

### BGP Anycast Configuration

For global deployments with anycast routing:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-anycast
spec:
  nodeType: Horizon
  network: Mainnet
  version: "v21.0.0"
  
  loadBalancer:
    enabled: true
    mode: BGP
    addressPool: "stellar-anycast-pool"
    loadBalancerIP: "192.0.2.100"
    externalTrafficPolicy: Local
    
    bgp:
      localASN: 64512
      peers:
        - address: "192.168.1.1"
          asn: 64513
          holdTime: 90
          keepaliveTime: 30
          gracefulRestart: true
          
      communities:
        - "64512:100"
        
      advertisement:
        aggregationLength: 32
        localPref: 100
        
      bfdEnabled: true
      bfdProfile: "stellar-bfd"
```

## BGP Configuration Parameters

### Peer Configuration

| Parameter | Description | Default |
|-----------|-------------|---------|
| `address` | IP address of BGP peer router | Required |
| `asn` | Peer's Autonomous System Number | Required |
| `port` | BGP port | 179 |
| `holdTime` | BGP hold time in seconds | 90 |
| `keepaliveTime` | BGP keepalive interval | 30 |
| `ebgpMultiHop` | Enable EBGP multi-hop | false |
| `gracefulRestart` | Enable graceful restart | true |
| `passwordSecretRef` | Secret reference for MD5 auth | None |

### Advertisement Configuration

| Parameter | Description | Default |
|-----------|-------------|---------|
| `aggregationLength` | IPv4 prefix length (CIDR) | 32 |
| `aggregationLengthV6` | IPv6 prefix length | 128 |
| `localPref` | BGP local preference | None |
| `nodeSelectors` | Limit announcing nodes | All nodes |

### Communities

BGP communities allow traffic engineering and policy application:

```yaml
bgp:
  communities:
    - "64512:100"    # Standard community
    - "64512:1000"   # Service tag
  largeCommunities:
    - "64512:1:100"  # Large community (RFC 8092)
```

## Global Discovery

Enable global discovery for cross-region coordination:

```yaml
globalDiscovery:
  enabled: true
  region: "us-east"
  zone: "us-east-1a"
  priority: 100
  topologyAwareHints: true
  
  externalDns:
    hostname: "horizon.stellar.example.com"
    ttl: 60
    provider: "route53"
```

### Topology-Aware Hints

When enabled, Kubernetes will route traffic to nodes in the same zone when possible, reducing cross-zone traffic costs.

### External DNS Integration

Automatically register DNS records for your nodes:

```yaml
externalDns:
  hostname: "horizon.stellar.example.com"
  ttl: 60
  annotations:
    external-dns.alpha.kubernetes.io/aws-weight: "100"
```

## Health Checks

The operator automatically configures health check ports for load balancer probes:

```yaml
loadBalancer:
  healthCheckEnabled: true
  healthCheckPort: 9100
```

## Multi-Region Deployment

For a global deployment, create nodes in each region with the same anycast IP:

### US East Region

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-us-east
spec:
  loadBalancer:
    loadBalancerIP: "192.0.2.100"  # Same IP
    bgp:
      localASN: 64512
      advertisement:
        localPref: 100  # Primary
```

### EU West Region

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-eu-west
spec:
  loadBalancer:
    loadBalancerIP: "192.0.2.100"  # Same IP
    bgp:
      localASN: 64513
      advertisement:
        localPref: 90   # Secondary
```

### Asia Pacific Region

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-ap-south
spec:
  loadBalancer:
    loadBalancerIP: "192.0.2.100"  # Same IP
    bgp:
      localASN: 64514
      advertisement:
        localPref: 80   # Tertiary
```

## Verification

### Check BGP Sessions

```bash
# View BGP peers
kubectl get bgppeers -n metallb-system

# Check MetalLB speaker logs
kubectl logs -n metallb-system -l app=metallb,component=speaker

# Verify IP allocation
kubectl get svc -n stellar-nodes -o wide
```

### Test Failover

```bash
# Scale down a node
kubectl scale deployment horizon-us-east -n stellar-nodes --replicas=0

# Verify traffic routes to next region
curl -v http://192.0.2.100:8000/
```

## Troubleshooting

### BGP Session Not Establishing

1. Verify firewall allows TCP port 179
2. Check ASN configuration matches peer router
3. Verify password if MD5 authentication is used
4. Check MetalLB speaker logs for errors

### IP Not Assigned

1. Verify IPAddressPool exists and has available IPs
2. Check BGPAdvertisement references correct pool
3. Ensure Service has correct annotations

### High Latency

1. Verify topology-aware hints are enabled
2. Check local preference values across regions
3. Ensure BFD is configured for fast failover

## Security Considerations

1. **Use MD5 Authentication**: Always configure BGP session passwords
2. **Restrict Peer IPs**: Only allow known router addresses
3. **Monitor BGP Sessions**: Alert on session flaps or hijacking attempts
4. **Prefix Filtering**: Configure routers to only accept expected prefixes

## References

- [MetalLB Documentation](https://metallb.universe.tf/)
- [BGP Configuration Guide](https://metallb.universe.tf/configuration/_advanced_bgp_configuration/)
- [Kubernetes Topology-Aware Hints](https://kubernetes.io/docs/concepts/services-networking/topology-aware-hints/)
- [External DNS](https://github.com/kubernetes-sigs/external-dns)
