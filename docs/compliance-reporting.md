# Automated Compliance Reporting

Continuous compliance monitoring and automated report generation for SOC 2, GDPR,
and PCI-DSS regulatory requirements.

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                  Compliance Reporting Pipeline                   │
│                                                                  │
│  Cluster State ──► Validation Pipeline (SOC2/GDPR/PCI-DSS)     │
│         │                    │                                   │
│         ▼                    ▼                                   │
│  Drift Detector ──► Evidence Collector ──► Report Generator     │
│         │                                        │               │
│         ▼                                        ▼               │
│  Compliance Monitor                    Export (PDF/JSON/CSV)    │
└─────────────────────────────────────────────────────────────────┘
```

## Supported Frameworks

| Framework | Rules | Key Controls |
|-----------|-------|-------------|
| SOC 2 Type II | CC6.1, CC6.6, CC7.2 | RBAC, mTLS, audit logging |
| GDPR | Art. 17, 25, 32 | Encryption, retention, PII scrubbing |
| PCI-DSS v4.0 | 3.4, 4.1, 10.2, 11.2 | Secret encryption, TLS, access logs |

## REST API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/v1/compliance/regulatory-report?format=json` | GET | Full compliance report (JSON) |
| `/api/v1/compliance/regulatory-report?format=pdf` | GET | PDF report |
| `/api/v1/compliance/regulatory-report?format=csv` | GET | CSV export |
| `/api/v1/compliance/status` | GET | Current compliance status |

## CLI Export

```bash
stellar-operator export-compliance --format json --output report.json
stellar-operator export-compliance --format pdf --output report.pdf
```

## Evidence Collection

Each rule validation produces a tamper-evident evidence artifact with SHA-256
content hash for audit trail integrity.

## Configuration Drift

The compliance monitor compares current cluster state against a baseline and
reports drift in security-critical settings (mTLS, RBAC, encryption, retention).
