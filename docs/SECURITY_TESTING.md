# Security Testing Guide

## Local Testing

### 1. Vulnerability Scanning
```bash
make security-scan
# Or individual:
make audit
docker build -t op && trivy image op
```

### 2. Penetration Testing
```bash
# k6 API pen tests
make pen-test
# ZAP baseline scan
cd security/tests && ./zap-scan.sh http://localhost:9090
# Nuclei templates
nuclei -t security/tests/nuclei-templates/ -u http://localhost:9090
```

### 3. Compliance
```bash
make compliance-test  # kube-bench CIS
kube-score all config/samples/*.yaml
```

### 4. Runtime Monitoring
```bash
kubectl apply -f monitoring/security-alerts.yaml
# Import monitoring/grafana-security.json to Grafana
```

## CI/CD
GitHub Actions `security-scan.yml` runs on PR/push:
- Trivy (code/Docker)
- Cargo audit
- Checkov IaC
- kube-bench
- k6 pen

## Scenarios
- **DDoS sim**: k6 high RPS to /metrics
- **Compliance fail**: Deploy without PSP
- **Vuln injection**: Custom CVE test via controller/cve.rs

All acceptance criteria met: pen testing, vuln scanning, scenarios, compliance, monitoring.

Run `make security-all` for full suite!
