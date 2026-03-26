# Container Image Pinning Best Practices

Using mutable tags like `latest` or generic version numbers (e.g., `v21`) in production environments introduces security and stability risks. This document outlines why you should use image digests and how to implement them with the Stellar-K8s operator.

## Why Pin by Digest?

1. **Security**: Tags can be moved to point to different images. An attacker who gains access to your container registry could overwrite a tag with a malicious image. Digests (`sha256:...`) are immutable and cryptographically linked to the image content.
2. **Reproducibility**: Different nodes in your cluster might pull the "same" tag at different times and end up with different versions of the software if the tag was updated in between.
3. **Rollback Safety**: When you roll back a deployment, you want to be 100% sure you're going back to the exact same bits that worked before.

## How to use Digests in Stellar-K8s

The `StellarNode` specification supports pinning images in the `version` field.

### 1. Pin by Digest Only
If you provide only the SHA256 digest, the operator will use it as the primary identifier.

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator
spec:
  nodeType: Validator
  network: Public
  version: "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678"
  # ... other fields
```

### 2. Pin by Tag and Digest (Recommended)
This format provides a human-readable tag for reference while enforcing the exact image via the digest.

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon
spec:
  nodeType: Horizon
  network: Public
  version: "v2.10.0@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678"
  # ... other fields
```

## Admission Webhook Warnings

The Stellar-K8s Validating Webhook will automatically issue a warning if a mutable tag is detected in your `StellarNode` manifest.

- **Warning**: `Mutable image tag 'v21.0.0' used. For production, it is recommended to pin the image by digest...`
- **Critical Warning**: `Using mutable tag 'latest' is a security risk...`

These warnings do not block the creation or update of the resource but serve as a reminder to follow production best practices.
