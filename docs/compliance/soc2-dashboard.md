# Compliance Reporting Dashboard for SOC2/ISO27001

## Overview
Automated auditing of the Stellar-K8s cluster against SOC2/ISO27001 security controls.

## Controls Monitored
- **Encryption at rest**: Validating PVCs and Secrets.
- **mTLS**: Ensuring sidecar injection and strict mTLS policies.
- **RBAC**: Auditing RoleBindings and ClusterRoleBindings.
- **Logging**: Verifying fluentd/promtail daemonsets are running.

## Features
- Provides a Compliance Gap Analysis summary.
- Generates time-stamped Audit Evidence reports.
- Extensible for custom compliance benchmarks via ConfigMaps.
