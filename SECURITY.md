# Security Policy

## Supported Versions

The following versions of Stellar-K8s currently receive security updates:-

| Version | Supported          | End of Support |
| ------- | ------------------ | -------------- |
| 0.1.x   | :white_check_mark: | TBD            |

> Only the latest patch release within a supported minor version receives security fixes.
> Users are strongly encouraged to stay on the latest release.

---

## Reporting a Vulnerability

**Do not report security vulnerabilities through public GitHub issues, pull requests, or discussions.**

### Option 1 — GitHub Private Security Advisory (Preferred)

Use GitHub's built-in private disclosure mechanism:

1. Go to the [Security Advisories](../../security/advisories/new) tab of this repository
2. Click **"Report a vulnerability"**
3. Fill in the details and submit

This keeps the report confidential and allows coordinated disclosure.

### Option 2 — Encrypted Email

Send an encrypted report to:

**security@stellar-k8s.io**

For encrypted communication, use our PGP public key:

```
-----BEGIN PGP PUBLIC KEY BLOCK-----

mQINBGYXXXXBEAC... (placeholder — replace with actual project PGP key)
-----END PGP PUBLIC KEY BLOCK-----
```

Key fingerprint: `XXXX XXXX XXXX XXXX XXXX  XXXX XXXX XXXX XXXX XXXX`

> Until a project PGP key is published, reporters may use GitHub's private advisory
> system (Option 1) or email **samuelotowo@gmail.com** directly.

---

## What to Include in Your Report

Please provide as much of the following as possible:

- Type of vulnerability (e.g., privilege escalation, SSRF, RCE, information disclosure)
- Affected component(s) and version(s)
- Full path(s) of relevant source files
- Step-by-step reproduction instructions
- Proof-of-concept or exploit code (if available)
- Potential impact and attack scenario
- Any suggested mitigations

---

## Response Timeline

| Stage                        | Target SLA         |
| ---------------------------- | ------------------ |
| Acknowledgment               | 48 hours           |
| Initial triage & severity    | 5 business days    |
| Fix development begins       | Based on severity  |
| Patch release (Critical/High)| 14 days            |
| Patch release (Medium/Low)   | 30–90 days         |
| Public disclosure            | After patch ships  |

Severity is assessed using [CVSS v3.1](https://www.first.org/cvss/calculator/3.1).

---

## Disclosure Policy

We follow [coordinated vulnerability disclosure](https://cheatsheetseries.owasp.org/cheatsheets/Vulnerability_Disclosure_Cheat_Sheet.html):

1. Reporter submits vulnerability privately
2. We validate, triage, and assign a CVE if warranted
3. We develop and test a fix in a private fork
4. A patched release is published
5. A GitHub Security Advisory is made public
6. Reporter is credited (unless they prefer anonymity)

We ask reporters to:
- Allow at least **90 days** before public disclosure
- Avoid accessing or modifying data beyond what is needed to demonstrate the issue
- Not disrupt production systems or other users

---

## Security Update Process

```
Report received
     │
     ▼
Acknowledgment (48h)
     │
     ▼
Triage & CVSS scoring
     │
     ├─ Invalid / Not a vulnerability ──► Close with explanation
     │
     ▼
Private fix branch + draft advisory
     │
     ▼
Patch release + CVE assignment
     │
     ▼
Public advisory + reporter credit
```

---

## Security Best Practices for Deployers

### Container Security
- Use the latest stable release; pin image digests in production
- Scan images with Trivy or Grype before deployment
- Operator containers run as non-root by default — do not override this

### RBAC & Permissions
- Follow least-privilege; review the generated RBAC manifests before applying
- Use dedicated service accounts per component
- Audit RBAC bindings regularly

### Network Security
- Enable mTLS for inter-component communication
- Apply Kubernetes NetworkPolicies to restrict operator traffic
- Protect the admission webhook endpoint with proper TLS certificates

### Secrets Management
- Use an external secrets manager (Vault, AWS Secrets Manager, etc.)
- Enable etcd encryption at rest
- Rotate secrets and TLS certificates regularly

### Monitoring
- Enable audit logging on the API server
- Alert on unexpected RBAC changes or CRD modifications
- Monitor operator metrics via the Prometheus endpoint

---

## Security Scanning in CI/CD

Our pipeline runs the following on every commit:

| Tool          | Purpose                                      |
| ------------- | -------------------------------------------- |
| `cargo audit` | Rust dependency advisory checks (RUSTSEC DB) |
| Trivy         | Container image vulnerability scanning       |
| Dependabot    | Automated dependency update PRs              |
| SBOM          | Software Bill of Materials generation        |

Results are uploaded to GitHub Security tab as SARIF reports.

---

## Known Security Considerations

### Admission Webhook
The WASM-based admission webhook validates all `StellarNode` resources. Ensure:
- The webhook TLS certificate is valid and rotated before expiry
- WASM plugins are loaded only from trusted, integrity-verified sources (SHA-256 checked)

### CRD Validation
Webhook validation prevents invalid configurations, resource exhaustion, and privilege escalation attempts via the `StellarNode` spec.

### REST API
The optional REST API should be protected by network policies and ingress authentication. Do not expose it publicly without authentication.

## Security Scanning & Testing

Our CI/CD pipeline includes comprehensive automated security testing:

### Vulnerability Scanning
- **Trivy** - Container/FS/dependency scanning (CRITICAL/HIGH alerts)
- **Cargo Audit** - Rust crates.io advisories
- **SBOM Generation** - CycloneDX supply chain transparency
- **CodeQL** - Semantic code analysis (GitHub Advanced Security)

### Penetration Testing
- **k6** - API load/penetration scenarios (DDoS, slowloris sim)
- **OWASP ZAP** - Baseline DAST for operator REST API
- **Nuclei** - Template-based vulnerability scanning

### Compliance Testing
- **kube-bench** - CIS Kubernetes Benchmark automation
- **Checkov** - IaC scanning for Helm charts/manifests
- **kube-score** - K8s resource scoring

Run locally: `make security-all`

## Runtime Security Monitoring

- **Prometheus + Grafana** - Security metrics dashboard
- **Falco** - Behavioral runtime security (planned integration)
- **Audit Logs** - API/server audit trails
- **CVE Auto-remediation** - Controller-based patching (src/controller/cve.rs)

- [CIS Kubernetes Benchmark](https://www.cisecurity.org/benchmark/kubernetes)
- [NIST SP 800-190 — Container Security](https://csrc.nist.gov/publications/detail/sp/800-190/final)
- [OWASP Kubernetes Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html)

---

## Contact

| Channel              | Address / Link                                          |
| -------------------- | ------------------------------------------------------- |
| Security advisories  | [GitHub Security Tab](../../security/advisories)        |
| Security email       | security@stellar-k8s.io                                 |
| Maintainer contact   | samuelotowo@gmail.com                                   |
| General issues       | [GitHub Issues](../../issues)                           |

---

## Attribution

We are grateful to security researchers who responsibly disclose vulnerabilities.
Reporters will be credited in the GitHub Security Advisory and CHANGELOG unless they
request anonymity.
