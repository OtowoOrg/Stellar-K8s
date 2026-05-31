# Ingress Configuration Guide

## Overview

The Stellar-K8s operator automates the creation of Kubernetes Ingress resources to expose Horizon and Soroban RPC nodes over HTTPS. Combined with **cert-manager**, it provides automatic TLS certificate provisioning and renewal using Let's Encrypt or custom Certificate Authorities.

## Features

✅ **Automatic Ingress Generation** - Creates Kubernetes Ingress resources with minimal configuration
✅ **TLS Certificate Management** - Integrates with cert-manager for automatic HTTPS setup
✅ **Let's Encrypt Support** - Provision free SSL/TLS certificates automatically
✅ **Custom Path Routing** - Route different paths to your Horizon/Soroban service
✅ **Multiple Ingress Controllers** - Compatible with NGINX, Traefik, and other controllers
✅ **mTLS & Custom CAs** - Support for mutual TLS and self-signed certificates

## Prerequisites

### 1. Kubernetes Cluster with Ingress Controller

Install an ingress controller (NGINX, Traefik, etc.):

```bash
# NGINX Ingress Controller (most common)
helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx
helm repo update
helm install nginx-ingress ingress-nginx/ingress-nginx \
  --namespace ingress-nginx \
  --create-namespace

# Traefik (alternative)
helm repo add traefik https://helm.traefik.io
helm install traefik traefik/traefik \
  --namespace traefik \
  --create-namespace
```

### 2. Install cert-manager

cert-manager automates TLS certificate provisioning:

```bash
# Install cert-manager CRDs
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.crds.yaml

# Install cert-manager
helm repo add jetstack https://charts.jetstack.io
helm install cert-manager jetstack/cert-manager \
  --namespace cert-manager \
  --create-namespace \
  --version v1.13.0
```

### 3. Create a cert-manager Issuer or ClusterIssuer

#### Let's Encrypt (Recommended for Public Domains)

```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-prod
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: admin@stellar.example.com
    privateKeySecretRef:
      name: letsencrypt-prod-key
    solvers:
      - http01:
          ingress:
            class: nginx
```

#### Self-Signed (Development/Internal)

```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: selfsigned-issuer
spec:
  selfSigned: {}
```

#### Custom CA (mTLS)

```yaml
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: custom-ca-issuer
  namespace: stellar-nodes
spec:
  ca:
    secretName: ca-key-pair  # Must contain tls.crt and tls.key
```

First, create the CA secret:

```bash
# Generate CA certificate and key
openssl genrsa -out ca.key 2048
openssl req -new -x509 -days 3650 -key ca.key -out ca.crt

# Create secret
kubectl create secret tls ca-key-pair \
  --cert=ca.crt \
  --key=ca.key \
  --namespace=stellar-nodes
```

## Basic Configuration

### Minimal Ingress Setup

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon
  namespace: stellar-nodes
spec:
  nodeType: Horizon
  network: Mainnet
  version: "v21.0.0"

  horizonConfig:
    databaseSecretRef: "horizon-db"
    enableIngest: true
    stellarCoreUrl: "http://stellar-core:11626"

  # Basic ingress configuration
  ingress:
    className: "nginx"
    hosts:
      - host: "horizon.example.com"
        paths:
          - path: "/"
            pathType: "Prefix"
    certManagerClusterIssuer: "letsencrypt-prod"
    tlsSecretName: "horizon-tls"
```

### Advanced Ingress Configuration

```yaml
ingress:
  # Ingress controller class
  className: "nginx"

  # DNS hosts and path-based routing
  hosts:
    - host: "horizon.example.com"
      paths:
        - path: "/"
          pathType: "Prefix"
        - path: "/metrics"
          pathType: "Exact"

    - host: "api.example.com"
      paths:
        - path: "/horizon"
          pathType: "Prefix"
        - path: "/soroban"
          pathType: "Prefix"

  # TLS configuration
  tlsSecretName: "horizon-tls"

  # cert-manager integration
  certManagerClusterIssuer: "letsencrypt-prod"  # Global issuer
  # OR
  certManagerIssuer: "custom-issuer"  # Namespaced issuer (takes precedence)

  # Custom annotations
  annotations:
    cert-manager.io/issue-temporary-certificate: "true"
    nginx.ingress.kubernetes.io/rate-limit: "1000"
    nginx.ingress.kubernetes.io/ssl-redirect: "true"
```

## DNS Configuration

After creating the Ingress, configure your DNS records to point to the ingress controller:

```bash
# Get the external IP or hostname of your ingress controller
kubectl get ingress -n stellar-nodes

# Example output:
# NAME                CLASS   HOSTS                   ADDRESS       PORTS   AGE
# horizon-ingress     nginx   horizon.example.com     203.0.113.42  80,443   2m

# Add DNS A record (if IP):
# horizon.example.com A 203.0.113.42

# Or CNAME (if hostname):
# horizon.example.com CNAME your-ingress.example.com
```

## Accessing Your Service

### Via HTTP (Before TLS Certificate is Ready)

```bash
# Port-forward to test
kubectl port-forward svc/horizon 8000:8000 -n stellar-nodes

# Access locally
curl http://localhost:8000
```

### Via HTTPS (After TLS Certificate is Provisioned)

```bash
# Check certificate status
kubectl get certificate -n stellar-nodes

# View certificate details
kubectl describe certificate horizon-tls -n stellar-nodes

# Access via HTTPS
curl https://horizon.example.com/
```

## Monitoring and Troubleshooting

### Check Ingress Status

```bash
kubectl describe ingress horizon-ingress -n stellar-nodes
kubectl get ingress -n stellar-nodes -w
```

### View cert-manager Events

```bash
# Check certificate provisioning status
kubectl describe certificate horizon-tls -n stellar-nodes

# View cert-manager logs
kubectl logs -n cert-manager -l app.kubernetes.io/name=cert-manager

# Check certificate secret
kubectl get secret horizon-tls -n stellar-nodes -o yaml
```

### Common Issues

#### Certificate Not Provisioning

```bash
# Check Certificate resource for errors
kubectl describe certificate horizon-tls -n stellar-nodes

# Verify issuer is accessible
kubectl describe clusterissuer letsencrypt-prod

# Check ACME challenge status
kubectl get orders,challenges -n stellar-nodes
```

#### Ingress Not Working

```bash
# Verify ingress controller is running
kubectl get pods -n ingress-nginx

# Check ingress controller logs
kubectl logs -n ingress-nginx -l app.kubernetes.io/name=ingress-nginx

# Test connectivity
kubectl run -it --rm debug --image=curlimages/curl --restart=Never -- \
  curl http://horizon-ingress.stellar-nodes.svc.cluster.local
```

#### DNS Not Resolving

```bash
# Verify DNS propagation
nslookup horizon.example.com

# Check from inside cluster
kubectl run -it --rm debug --image=alpine --restart=Never -- \
  nslookup horizon.example.com

# Verify ingress external IP is assigned
kubectl get ingress -n stellar-nodes -o wide
```

## Security Best Practices

### 1. Enforce HTTPS

```yaml
annotations:
  nginx.ingress.kubernetes.io/ssl-redirect: "true"
  nginx.ingress.kubernetes.io/force-ssl-redirect: "true"
```

### 2. Rate Limiting

```yaml
annotations:
  nginx.ingress.kubernetes.io/rate-limit: "100"  # requests per second
  nginx.ingress.kubernetes.io/limit-connections: "10"
```

### 3. Network Policies

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: horizon-allow-ingress
  namespace: stellar-nodes
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: stellar-node
  policyTypes:
    - Ingress
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              name: ingress-nginx
      ports:
        - port: 8000
          protocol: TCP
```

### 4. TLS Versions

```yaml
annotations:
  nginx.ingress.kubernetes.io/ssl-protocols: "TLSv1.2 TLSv1.3"
  nginx.ingress.kubernetes.io/ssl-ciphers: "HIGH:!aNULL:!MD5"
```

## Examples

See [ingress-example.yaml](../examples/ingress-example.yaml) for complete working examples including:

- Horizon with Let's Encrypt TLS
- Soroban RPC with mTLS setup
- Multiple host routing
- Autoscaling with ingress

## API Reference

### IngressConfig

```rust
pub struct IngressConfig {
    /// Ingress controller class name (e.g., "nginx", "traefik")
    pub class_name: Option<String>,

    /// Host rules with paths
    pub hosts: Vec<IngressHost>,

    /// TLS secret name for the certificate
    pub tls_secret_name: Option<String>,

    /// cert-manager namespaced issuer
    pub cert_manager_issuer: Option<String>,

    /// cert-manager cluster issuer
    pub cert_manager_cluster_issuer: Option<String>,

    /// Additional annotations
    pub annotations: Option<BTreeMap<String, String>>,
}
```

### IngressHost

```rust
pub struct IngressHost {
    /// DNS hostname
    pub host: String,

    /// HTTP paths for this host
    pub paths: Vec<IngressPath>,
}
```

### IngressPath

```rust
pub struct IngressPath {
    /// URL path (e.g., "/", "/api")
    pub path: String,

    /// Path type: "Prefix" or "Exact"
    pub path_type: Option<String>,
}
```

## Related Resources

- [Kubernetes Ingress Documentation](https://kubernetes.io/docs/concepts/services-networking/ingress/)
- [cert-manager Documentation](https://cert-manager.io/docs/)
- [NGINX Ingress Controller](https://kubernetes.github.io/ingress-nginx/)
- [Traefik Ingress Controller](https://doc.traefik.io/traefik/providers/kubernetes-ingress/)
