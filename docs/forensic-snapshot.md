# Forensic snapshot (operator-driven)

Use this flow when a validator misbehaves and you need a **60s PCAP**, optional **core dump** (with `enableShareProcessNamespace`), and an **SSE-KMS encrypted tarball in S3**.

## Prerequisites

- Kubernetes **1.23+** (ephemeral containers).
- Operator RBAC includes `pods/ephemeralcontainers` patch (Helm chart updated).
- IRSA or a Secret with `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` if not using node/instance roles.

## StellarNode spec

```yaml
spec:
  forensicSnapshot:
    s3Bucket: my-org-stellar-forensics
    s3Prefix: prod/validator-a
    kmsKeyId: arn:aws:kms:us-east-1:123456789012:key/...
    credentialsSecretRef: stellar-forensic-aws
    enableShareProcessNamespace: true   # recommended for gcore of stellar-core
```

## Trigger

```bash
kubectl annotate stellarnode prod-validator \
  stellar.org/request-forensic-snapshot=true --overwrite
```

Watch `status.forensicSnapshotPhase` (`Capturing` → `Complete`). The request annotation is cleared on success.

## Security

- Bundles may contain sensitive ledger / key material; restrict S3 bucket policies and use SSE-KMS.
- Prefer a dedicated forensic IAM role with `s3:PutObject` only on the forensic prefix.
