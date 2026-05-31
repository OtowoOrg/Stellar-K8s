# Automated Secret Rotation for Database Credentials

## Overview

Stellar-K8s provides automated rotation of PostgreSQL database passwords for Stellar Core and Horizon nodes, ensuring zero-downtime credential updates and enhanced security posture.

## Features

- **Automated Password Generation**: Cryptographically secure random passwords
- **Zero-Downtime Updates**: Coordinated updates to database and Kubernetes secrets
- **Rolling Pod Restarts**: Automatic pod restarts to pick up new credentials
- **Configurable Schedule**: Cron-based rotation schedule (monthly, quarterly, etc.)
- **Audit Logging**: Complete audit trail of all rotation events
- **Rollback Support**: Automatic rollback in case of failures
- **Webhook Notifications**: Real-time alerts for rotation events

## Architecture

The secret rotation process follows these steps:

1. **Generate New Password**: Create a cryptographically secure random password
2. **Update Database**: Execute `ALTER USER` command to update the database password
3. **Update Kubernetes Secret**: Patch the Kubernetes Secret with the new password
4. **Trigger Rolling Restart**: Add annotation to StatefulSet to trigger pod restart
5. **Verify Connectivity**: Test database connection with new credentials
6. **Log Event**: Record rotation event for audit trail

```
┌─────────────────────────────────────────────────────────────┐
│                    Secret Rotation Flow                      │
└─────────────────────────────────────────────────────────────┘

  ┌──────────────┐
  │   Scheduler  │  (Cron-based)
  └──────┬───────┘
         │
         ▼
  ┌──────────────────┐
  │ Generate Password│
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Update Database  │  (ALTER USER)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │  Update Secret   │  (Kubernetes API)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │  Restart Pods    │  (Rolling Update)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Verify & Audit   │
  └──────────────────┘
```

## Configuration

### Basic Configuration

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-horizon
  namespace: stellar
spec:
  nodeType: Horizon
  network: Testnet
  version: "v21.0.0"

  database:
    host: postgres.stellar.svc.cluster.local
    port: 5432
    database: horizon
    user: horizon
    passwordSecret: horizon-db-credentials

  secretRotation:
    enabled: true
    schedule: "0 0 1 * *" # Monthly on the 1st
    passwordLength: 32
    auditLoggingEnabled: true
```

### Advanced Configuration

```yaml
secretRotation:
  enabled: true

  # Rotation schedule (cron format)
  schedule: "0 0 1 */3 *" # Quarterly rotation

  # Password length (default: 32)
  passwordLength: 40

  # Database connection timeout in seconds
  dbTimeoutSeconds: 30

  # Maximum retry attempts
  maxRetries: 3

  # Enable audit logging
  auditLoggingEnabled: true

  # External audit log destination
  auditLogDestination: "https://audit-logs.example.com/api/events"

  # Notification webhook
  notificationWebhook: "https://slack.example.com/hooks/secret-rotation"
```

## Rotation Schedules

Common rotation schedules using cron syntax:

| Schedule   | Cron Expression | Description                          |
| ---------- | --------------- | ------------------------------------ |
| Monthly    | `0 0 1 * *`     | First day of every month at midnight |
| Bi-monthly | `0 0 1 */2 *`   | Every 2 months on the 1st            |
| Quarterly  | `0 0 1 */3 *`   | Every 3 months on the 1st            |
| Weekly     | `0 0 * * 0`     | Every Sunday at midnight             |
| Custom     | `0 2 15 * *`    | 15th of every month at 2 AM          |

## Audit Logging

All rotation events are logged with the following information:

```json
{
  "timestamp": "2026-04-25T10:30:00Z",
  "namespace": "stellar",
  "nodeName": "my-horizon",
  "databaseUser": "horizon",
  "secretName": "horizon-db-credentials",
  "status": "completed",
  "passwordHash": "a3f5b8c9d2e1f4a7b6c5d8e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0"
}
```

### Rotation Statuses

- `started`: Rotation process initiated
- `passwordGenerated`: New password generated
- `databaseUpdated`: Database password updated
- `secretUpdated`: Kubernetes Secret updated
- `podsRestarted`: Pods restarted with new credentials
- `completed`: Rotation completed successfully
- `failed`: Rotation failed (with error message)
- `rolledBack`: Rotation rolled back due to failure

## Security Considerations

### Password Generation

Passwords are generated using cryptographically secure random number generation:

- Uses `rand::thread_rng()` with `Alphanumeric` distribution
- Default length: 32 characters (configurable)
- Character set: `[A-Za-z0-9]`

### Password Storage

- Passwords are stored in Kubernetes Secrets (base64 encoded)
- Audit logs contain SHA256 hashes, not plaintext passwords
- Database connections use TLS encryption (recommended)

### Access Control

Ensure proper RBAC permissions for the operator:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: stellar-operator-secret-rotation
  namespace: stellar
rules:
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "list", "patch", "update"]
  - apiGroups: ["apps"]
    resources: ["statefulsets"]
    verbs: ["get", "list", "patch", "update"]
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes"]
    verbs: ["get", "list", "watch"]
```

## Rollback Procedure

If secret rotation fails after updating the database but before updating the Kubernetes Secret, the operator automatically attempts to rollback:

1. Detect failure during Secret update
2. Reconnect to database with old password
3. Execute `ALTER USER` to restore old password
4. Log rollback event
5. Send notification

## Monitoring

### Prometheus Metrics

The operator exposes the following metrics:

```
# Total number of secret rotations
stellar_operator_secret_rotations_total{namespace, node_name, status}

# Duration of secret rotation in seconds
stellar_operator_secret_rotation_duration_seconds{namespace, node_name}

# Last successful rotation timestamp
stellar_operator_secret_rotation_last_success_timestamp{namespace, node_name}
```

### Grafana Dashboard

Import the secret rotation dashboard from `monitoring/secret-rotation-dashboard.json`:

- Rotation success rate
- Rotation duration (p50, p95, p99)
- Failed rotations by namespace
- Time since last successful rotation

## Troubleshooting

### Common Issues

#### 1. Database Connection Timeout

**Symptom**: Rotation fails with "Failed to connect to database"

**Solution**:

- Verify database is accessible from operator pod
- Check `dbTimeoutSeconds` configuration
- Verify database credentials in Secret

#### 2. Insufficient Permissions

**Symptom**: Rotation fails with "ALTER USER failed"

**Solution**:

- Ensure database user has `CREATEROLE` or `SUPERUSER` privilege
- Grant necessary permissions: `ALTER USER horizon WITH CREATEROLE;`

#### 3. Pod Restart Failure

**Symptom**: Rotation completes but pods don't restart

**Solution**:

- Check operator has permissions to patch StatefulSets
- Verify StatefulSet exists and is managed by operator
- Check for PodDisruptionBudget blocking restarts

### Manual Rotation

To trigger a manual rotation outside the schedule:

```bash
# Add annotation to trigger immediate rotation
kubectl annotate stellarnode my-horizon \
  stellar.org/rotate-secret=true \
  -n stellar
```

### Verify Rotation Status

```bash
# Check operator logs
kubectl logs -n stellar-system \
  -l app=stellar-operator \
  --tail=100 | grep "secret rotation"

# Check audit logs
kubectl logs -n stellar-system \
  -l app=stellar-operator \
  --tail=100 | grep "AUDIT"
```

## Best Practices

1. **Test in Non-Production First**: Validate rotation process in test environments
2. **Monitor Rotation Events**: Set up alerts for failed rotations
3. **Regular Schedule**: Rotate credentials at least quarterly
4. **Audit Trail**: Enable audit logging for compliance
5. **Backup Secrets**: Keep encrypted backups of Kubernetes Secrets
6. **Use Strong Passwords**: Set `passwordLength` to at least 32 characters
7. **TLS Connections**: Always use TLS for database connections
8. **Notification Webhooks**: Configure webhooks for real-time alerts

## Compliance

Secret rotation helps meet compliance requirements:

- **PCI DSS**: Requirement 8.2.4 (Change passwords every 90 days)
- **SOC 2**: Access control and credential management
- **HIPAA**: Technical safeguards for access control
- **ISO 27001**: A.9.4.3 (Password management system)

## Examples

See `examples/secret-rotation-example.yaml` for complete configuration examples.

## Related Documentation

- [Backup Verification](backup-verification.md)
- [Security Best Practices](security/best-practices.md)
- [Database Management](database-management.md)
