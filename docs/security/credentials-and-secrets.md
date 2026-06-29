# Credentials and Secrets Management

Central reference for all credential and secret management documentation in Stellar-K8s.

## Overview

Stellar-K8s supports multiple strategies for managing sensitive credentials, from basic Kubernetes Secrets to external KMS providers with automatic rotation. The right choice depends on your security requirements, compliance needs, and operational maturity.

## Quick Reference

| Topic | Doc | Use case |
|---|---|---|
| Basic secret management | [Secret Management Guide](../secret-management-guide.md) | Getting started with `StellarSecret` CRD |
| KMS integration | [Advanced Secret Management with KMS](../secret-management-kms.md) | AWS KMS, Azure Key Vault, GCP Cloud KMS |
| Automated rotation | [Secret Rotation](../secret-rotation.md) | Zero-downtime database credential rotation |
| HashiCorp Vault | [Vault + Stellar Tutorial](../vault-stellar-tutorial.md) | Production Vault Agent Injector pattern |
| Production hardening | [Security Hardening Guide](../production-security-hardening.md) | Full security posture for production |
| External Secrets Operator | [ExternalSecret chart template](../../charts/stellar-operator/templates/externalsecret.yaml) | ESO integration via Helm |

## Secret Types

| Secret | Storage | Rotation | Source |
|---|---|---|---|
| Validator seed (`STELLAR_SEED`) | Kubernetes Secret / Vault | Manual or `vaultRef` | Wallet / Stellar Core |
| Database credentials (Horizon / Core) | Kubernetes Secret | Automatic (cron-based) | Generated / Provided |
| mTLS certificates | Kubernetes Secret (operator-managed) | Automatic renewal | Operator CA |
| Webhook HMAC key | Kubernetes Secret | Manual | Deployer |
| API tokens / OIDC secrets | Kubernetes Secret / ExternalSecret | Manual or ESO | Identity provider |
| S3 / cloud credentials | Kubernetes Secret / IRSA | Cloud IAM rotation | Cloud provider |

## Architecture Overview

```
                    ┌──────────────────────────────────┐
                    │       Credential Sources          │
                    ├──────────────────────────────────┤
                    │  Kubernetes Secrets  (static)     │
                    │  External Secrets    (ESO)        │
                    │  Vault Agent         (injector)   │
                    │  KMS backends (AWS/Azure/GCP)    │
                    └──────────┬───────────────────────┘
                               │
                    ┌──────────▼───────────────────────┐
                    │       Consumption Methods         │
                    ├──────────────────────────────────┤
                    │  Environment variables            │
                    │  Volume mounts (files)            │
                    │  Sidecar injection                │
                    │  Direct API (Vault Agent)         │
                    └──────────┬───────────────────────┘
                               │
                    ┌──────────▼───────────────────────┐
                    │      Lifecycle Management         │
                    ├──────────────────────────────────┤
                    │  Automatic rotation (cron)        │
                    │  Zero-downtime updates            │
                    │  Version rollback                 │
                    │  Immutable audit trail            │
                    └──────────────────────────────────┘
```

## Security Principles

1. **Secrets are never logged** — The operator redacts secret values from all log output
2. **Least privilege** — Each component accesses only the secrets it needs
3. **Encryption at rest** — Kubernetes Secrets are encrypted at the etcd level (recommended)
4. **Encryption in transit** — All secret delivery uses TLS
5. **Rotation** — Automated rotation limits the blast radius of credential leaks
6. **Audit** — All secret access and rotation events are logged immutably

## Compliance Mapping

| Standard | Requirement | How Stellar-K8s addresses it |
|---|---|---|
| SOC 2 | Access control, credential management | RBAC + audit logging + rotation |
| PCI DSS 8.2.4 | Change passwords every 90 days | Configurable cron-based rotation |
| HIPAA §164.312 | Technical safeguards for access control | mTLS + KMS encryption + audit |
| ISO 27001 A.9.4.3 | Password management system | Automated rotation + vault integration |

## Related Documentation

- [Production Security Hardening](../production-security-hardening.md)
- [mTLS Guide](../mtls-guide.md)
- [Pod Security Standards](pss.md)
- [Gatekeeper Policies](../gatekeeper-policies.md)
- [Image Pinning](../image-pinning.md)
