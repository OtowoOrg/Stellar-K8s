# Container Image Security

*Addresses issues #1334 and #1420.*

This document covers the container image vulnerability scanning pipeline,
Cosign supply-chain signing, and the deployment gate policy.

---

## Pipeline Overview

The [`.github/workflows/container-image-security.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/container-image-security.yml)
workflow runs on every PR, every push to `main`, and nightly.

```
build-image ──► trivy-scan  ──► sign-image ──► verify-signature
            └─► grype-scan  ─┘
            └─► sbom
```

| Job | Purpose | Blocks merge? |
|---|---|---|
| `build-image` | Build the operator image | Yes (prerequisite) |
| `trivy-scan` | Vulnerability scan (CRITICAL gate) | **Yes** |
| `grype-scan` | Cross-validation scan | Reported only |
| `sbom` | SPDX SBOM generation | No |
| `sign-image` | Keyless Cosign signing | No (post-merge) |
| `verify-signature` | Signature retrievability check | No (post-merge) |

---

## Vulnerability Scanning

### Trivy (primary gate)

Trivy runs two passes on every PR:

1. **Report pass** — scans for `CRITICAL` and `HIGH` severity CVEs and uploads
   a SARIF file to the GitHub Security tab.
2. **Gate pass** — scans for `CRITICAL` CVEs that have an upstream fix
   available (`ignore-unfixed: true`).  The job exits with code `1` if any
   are found, **blocking the merge**.

The `ignore-unfixed` flag prevents base-image CVEs with no available fix from
wedging every PR in the repository.  Those CVEs are still reported in the
Security tab by the report pass.

### Grype (cross-validation)

Grype from Anchore provides an independent vulnerability database.  Its
findings are uploaded to the Security tab as a separate SARIF category.
Grype does not gate merges on its own — if Grype finds a critical that Trivy
misses, a maintainer should open a security issue and check for a Trivy DB
update within 48 hours.

### Nightly scans

The workflow runs nightly at 04:00 UTC to detect newly published CVEs against
the latest `main` image, independent of code changes.

---

## SBOM (Software Bill of Materials)

[Syft](https://github.com/anchore/syft) generates an SPDX-JSON SBOM on every
run.  The SBOM artifact (`stellar-operator-sbom.spdx.json`) is attached to
the workflow run and can be used for:

- Automated license compliance auditing.
- Offline CVE scanning (export and run `grype sbom:stellar-operator-sbom.spdx.json`).
- Supply-chain attestation alongside the Cosign signature.

---

## Image Signing (Cosign)

Images published from `main` are signed keylessly via
[Sigstore Cosign](https://docs.sigstore.dev/cosign/signing/overview/).

### Why keyless signing?

The signing identity is the GitHub Actions OIDC token, so there is no
long-lived private key to store, rotate, or leak.  The signature is anchored
to the repository and workflow path in the Rekor transparency log.

### Signed by digest, not tag

Images are signed using their `sha256:...` digest.  A tag can be repointed to
different content at any time; a digest signature proves exactly *what* you
pulled.

### Verifying an image

```bash
cosign verify \
  --certificate-identity-regexp "^https://github.com/OtowoOrg/Stellar-K8s/" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  ghcr.io/otoworg/stellar-operator:0.1.0
```

The `--certificate-identity-regexp` flag is required.  Without it, `cosign
verify` accepts any Fulcio identity that signed the digest — which is not what
you want.

---

## Deployment Gate Policy

| Condition | Effect |
|---|---|
| CRITICAL CVE with available fix | **CI fails** — merge blocked |
| CRITICAL CVE with no fix | Reported in Security tab; merge not blocked |
| HIGH CVE | Reported in Security tab; merge not blocked |
| Unsigned image | Admission webhook rejects pod (if policy enforced) |

### Admission Webhook Policy

To enforce at deploy time, add a [Kyverno](https://kyverno.io/) or
[OPA/Gatekeeper](https://open-policy-agent.github.io/gatekeeper/) policy that
verifies the Cosign signature before admitting operator pods.  An example
Kyverno policy:

```yaml
apiVersion: kyverno.io/v1
kind: ClusterPolicy
metadata:
  name: verify-stellar-operator-signature
spec:
  validationFailureAction: Enforce
  rules:
    - name: check-image-signature
      match:
        resources:
          kinds: [Pod]
          namespaces: [stellar-system]
      verifyImages:
        - imageReferences:
            - "ghcr.io/otoworg/stellar-operator:*"
          attestors:
            - entries:
                - keyless:
                    subject: "https://github.com/OtowoOrg/Stellar-K8s/.github/workflows/container-image-security.yml@refs/heads/main"
                    issuer: "https://token.actions.githubusercontent.com"
```

---

## Summary of Security-Scan Actions

| Action | Path | Purpose |
|---|---|---|
| `security-scan` | `.github/actions/security-scan` | Reusable Trivy action with SARIF + gate |
| `sign-image` | `.github/actions/sign-image` | Keyless Cosign signing by digest |

Both actions are reused by `release.yml` so the same gate and signing
behaviour applies to production releases.
