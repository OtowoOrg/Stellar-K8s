# Certificate Management Setup Guide

This guide covers the automated TLS certificate lifecycle management for Stellar-K8s, including cert-manager deployment, issuer configuration, certificate provisioning, renewal, and expiry monitoring.

## Overview

Stellar-K8s uses cert-manager for automated TLS certificate management. The operator supports:

- **Let's Encrypt** (ACME HTTP-01) for public-facing certificates
- **Vault PKI** for internal/private CA certificates
- **Local CA** for development/testing environments

## Prerequisites

- Kubernetes 1.28+
- cert-manager installed (or enabled via Helm chart)
- Ingress controller (NGINX or Traefik)
- Prometheus Operator (for expiry monitoring)

## Quick Start

### 1. Enable cert-manager via Helm

```bash
helm install stellar-operator ./charts/stellar-operator \
  --set certManagement.enabled=true \
  --set certManagement.backend=cert-manager \
  --set certManagement.acmeEmail=admin@stellar.org
```

### 2. Install cert-manager (if not already installed)

```bash
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.14.0/cert-manager.yaml
```

Or via Helm:

```bash
helm repo add jetstack https://charts.jetstack.io
helm install cert-manager jetstack/cert-manager \
  --namespace cert-manager \
  --create-namespace \
  --set installCRDs=true
```

### 3. Verify ClusterIssuer

```bash
kubectl get clusterissuer letsencrypt-prod -o yaml
```

## Configuration

### Helm Values

```yaml
certManagement:
  enabled: true
  backend: "cert-manager"  # or "vault-pki", "local-ca"
  acmeEmail: "admin@stellar.org"
  ingressClass: "nginx"
  expiryAlerts:
    warningDays: 30
    criticalDays: 7
    emergencyHours: 24
  defaultTtlHours: 2160  # 90 days
  renewBeforeDays: 30
  certificates:
    - name: horizon-tls
      namespace: stellar
      commonName: horizon.stellar.org
      dnsNames:
        - horizon.stellar.org
        - api.stellar.org
    - name: validator-tls
      namespace: stellar
      commonName: validator.stellar.org
      dnsNames:
        - validator.stellar.org
        - stellar-validator-0.stellar.svc.cluster.local
```

### Certificate Resources

Each certificate entry creates a `Certificate` resource:

```yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: stellar-horizon-tls
  namespace: stellar
spec:
  secretName: stellar-horizon-tls
  issuerRef:
    name: letsencrypt-prod
    kind: ClusterIssuer
  commonName: horizon.stellar.org
  dnsNames:
    - horizon.stellar.org
    - api.stellar.org
  duration: 2160h  # 90 days
  renewBefore: 720h  # 30 days
  privateKey:
    algorithm: ECDSA
    size: 256
```

## Automatic Renewal

cert-manager automatically renews certificates before expiration. The `renewBefore` field controls when renewal is triggered:

- **30 days before expiry:** Renewal process begins
- **New certificate issued:** Via ACME challenge
- **TLS secret updated:** Kubernetes secret is updated with new cert/key
- **Zero-downtime:** Services continue using old cert until new one is ready

No manual intervention is required for renewal.

## Expiry Monitoring

### Prometheus Alerts

When `monitoring.prometheusRule.certAlerts.enabled=true`, the following alerts are created:

| Alert | Condition | Severity |
|-------|-----------|----------|
| `CertificateExpiringSoon` | < 30 days to expiry | Warning |
| `CertificateExpiringCritical` | < 7 days to expiry | Critical |
| `CertificateExpired` | Past expiry | Critical |
| `CertificateRenewalFailed` | Ready=False for 15m | Warning |
| `ACMEAccountRegistrationFailed` | Registration failure | Critical |

### Verify Certificate Status

```bash
# Check certificate status
kubectl get certificates -n stellar

# Check certificate details
kubectl describe certificate stellar-horizon-tls -n stellar

# Check certificate expiration
kubectl get certificate stellar-horizon-tls -n stellar -o jsonpath='{.status.notAfter}'

# Check cert-manager logs for issues
kubectl logs -n cert-manager -l app=cert-manager
```

### Manual Renewal

```bash
# Force certificate renewal
kubectl cert-manager renew stellar-horizon-tls -n stellar
```

## Vault PKI Backend

For internal/private CA certificates:

```yaml
certManagement:
  enabled: true
  backend: "vault-pki"
  vault:
    addr: "https://vault.stellar-system.svc:8200"
    pkiMount: "pki_int"
    roleName: "stellar-operator"
    tokenSecretName: "vault-token"
    tlsVerify: true
```

### Vault Setup

1. Enable PKI secrets engine:
   ```bash
   vault secrets enable pki
   vault secrets tune -max-lease-ttl=87600h pki
   ``

2. Create a role:
   ```bash
   vault write pki/roles/stellar-operator \
     allowed_domains="stellar.org,stellar-system.svc" \
     allow_subdomains=true \
     max_ttl=8760h
   ```

3. Create a Kubernetes secret with the Vault token:
   ```bash
   kubectl create secret generic vault-token \
     --from-literal=token=<VAULT_TOKEN>
   ```

## Troubleshooting

### Certificate Not Issuing

1. Check cert-manager logs:
   ```bash
   kubectl logs -n cert-manager -l app=cert-manager --tail=100
   ```

2. Check certificate events:
   ```bash
   kubectl describe certificate stellar-horizon-tls -n stellar
   ```

3. Verify ClusterIssuer is ready:
   ```bash
   kubectl get clusterissuer letsencrypt-prod -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}'
   ```

### ACME Challenge Failing

1. Verify ingress controller is running:
   ```bash
   kubectl get pods -n ingress-nginx
   ```

2. Check ACME challenge pods:
   ```bash
   kubectl get pods -n cert-manager -l app=cm-acme-http-solver
   ```

3. Verify DNS resolution for your domain

### Renewal Failures

1. Check cert-manager events:
   ```bash
   kubectl get events -n cert-manager --field-selector reason=RenewFailed
   ```

2. Manually trigger renewal:
   ```bash
   kubectl cert-manager renew <certificate-name> -n <namespace>
   ```

## Security Considerations

- Private keys are stored in Kubernetes Secrets (not committed to Git)
- ACME account keys are stored in ClusterIssuer resources
- Certificate secrets should be restricted via RBAC
- Enable cert-manager webhook for admission control
- Monitor certificate expiry alerts to prevent service outages

## References

- [cert-manager documentation](https://cert-manager.io/docs/)
- [Let's Encrypt ACME protocol](https://letsencrypt.org/how-it-works/)
- [Vault PKI secrets engine](https://developer.hashicorp.com/vault/docs/secrets/pki)
