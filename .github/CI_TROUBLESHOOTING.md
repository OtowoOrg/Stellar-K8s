# CI/CD Troubleshooting Guide

## Quick Fixes for Common CI Failures

### Docker Build Failures

**Error: "invalid digest format"**
```
Error: failed to solve: debian:bookworm-slim@sha256:1234567890...: invalid digest format
```
**Fix:** The base image digest is invalid. Update `Dockerfile` line 68 with current digest:
```bash
# Get current digest
docker pull debian:bookworm-slim
docker inspect debian:bookworm-slim | grep -A1 RepoDigests

# Update Dockerfile
FROM debian:bookworm-slim@sha256:<actual-digest> AS runtime-base
```

**Error: "permission denied" during Docker build**
```
Error: failed to solve: process "/bin/sh -c cargo build --release" did not complete successfully: exit code 1
```
**Fix:** Check buildx setup and cache permissions:
```bash
docker buildx ls
docker system prune -f
```

### Security Audit Failures

**Error: "cargo audit failed with advisories"**
**Fix:** Review `.cargo/audit.toml` and either:
1. Add justification for new advisory
2. Upgrade affected dependency
3. Remove ignore if advisory is resolved

**Process:**
1. Run `cargo audit` locally to see exact advisory
2. Check upstream fix availability
3. Document rationale in `audit.toml` if no fix exists

### Test Failures

**Error: "Chaos test timeout"**
```
Error: timed out waiting for condition on pods/stellar-operator
```
**Fix:** Check cluster resources and increase timeout:
```bash
kubectl get pods -n stellar-operator-system
kubectl describe pod <pod-name> -n stellar-operator-system
kubectl logs <pod-name> -n stellar-operator-system
```

**Error: "Benchmark regression detected"**
**Fix:** Review performance changes:
```bash
# Compare with baseline
git diff HEAD~1 -- benchmarks/baselines/
# Run local benchmark
cd benchmarks && ./run-regression-test.sh
```

### Cache Issues

**Error: "Rust cache restore failed"**
**Fix:** Clear cache and retry:
```yaml
# In GitHub Actions UI: Clear cache for 'rust-cache-<key>'
# Or wait for cache expiration (7 days)
```

**Error: "Docker layer cache miss"**
**Fix:** Verify cache configuration in workflow:
```yaml
cache-from: type=gha
cache-to: type=gha,mode=max
```

## Reproducing CI Failures Locally

### 1. Docker Build Issues
```bash
# Reproduce exact CI build
docker build --target runtime --platform linux/amd64 .

# Debug layer by layer
docker build --target builder .
docker run -it <builder-image-id> /bin/bash
```

### 2. Rust Test Failures
```bash
# Use same environment as CI
cargo test --all-features --workspace
cargo clippy --all-features --workspace -- -D warnings
cargo audit
```

### 3. Helm Template Issues
```bash
# Validate templates like CI does
helm template stellar-operator charts/stellar-operator
helm lint charts/stellar-operator
```

### 4. Performance Regression
```bash
# Run same benchmark as CI
cd benchmarks
./run-regression-test.sh
```

## Debugging Specific GitHub Actions Runs

### Getting Run Details
```bash
# Install GitHub CLI
gh auth login

# Get run details
gh run list --repo stellar/stellar-k8s --limit 10
gh run view <run-id> --log

# Download artifacts
gh run download <run-id>
```

### Common Run ID Investigation Steps
1. Check which jobs failed: `gh run view <run-id>`
2. Download logs: `gh run view <run-id> --log > ci-logs.txt`
3. Look for specific errors in logs
4. Compare with successful runs: `gh run list --json --jq`

## Action Version Management

### Current Pinned Versions (Updated 2026-07-28)
- `actions/checkout`: `@v7`
- `actions/setup-python`: `@v6`  
- `docker/build-push-action`: `@v7`
- `aquasecurity/trivy-action`: `@v0.36.0`
- `Swatinem/rust-cache`: `@v2`

### Updating Action Versions
```bash
# Check for updates
gh api repos/actions/checkout/releases/latest
gh api repos/docker/build-push-action/releases/latest

# Update in all workflows
find .github/workflows -name "*.yml" -exec sed -i 's/@v6/@v7/g' {} +
```

## Security Audit Management

### Adding New Ignores
1. **Never** add ignores without tracking issue
2. Document rationale in `.cargo/audit.toml`
3. Set review date
4. Link to upstream tracking issue

### Template for New Ignores
```toml
# <crate-name> <version> – <brief-description> (RUSTSEC-YYYY-XXXX).
# <detailed-explanation-of-why-ignored>
# <conditions-for-removal>
# Re-evaluate: <date>
"RUSTSEC-YYYY-XXXX",
```

## Performance Regression Thresholds

### Current Thresholds
- **Operator startup**: ±5% from baseline
- **Webhook latency**: ±10% from baseline  
- **Memory usage**: ±15% from baseline

### Tuning Thresholds
Edit `benchmarks/scripts/compare_benchmarks.py`:
```python
THRESHOLDS = {
    'startup_time': 0.05,  # 5%
    'webhook_p99': 0.10,   # 10%
    'memory_peak': 0.15    # 15%
}
```

## Escalation Process

### Level 1: Self-Service (Use this guide)
- Common failures with known fixes
- Action version updates
- Cache clearing

### Level 2: Team Review
- New security advisories requiring evaluation
- Performance regression analysis
- Infrastructure-level failures

### Level 3: Platform Team
- GitHub Actions service issues
- Registry/authentication problems  
- Quota/billing issues

## Monitoring and Alerts

### Key Metrics to Watch
- **Success rate**: >95% on main branch
- **Duration**: CI should complete within 45 minutes
- **Cache hit rate**: >80% for Rust builds

### Setting Up Alerts
Monitor these workflow conclusion events:
- `workflow_run.completed` with `conclusion: failure`
- Pattern: >3 consecutive failures on main

## Recent Changes Log

### 2026-07-28: Pipeline Hardening
- ✅ Fixed invalid Docker base image digest
- ✅ Centralized security audit configuration
- ✅ Standardized action versions across workflows
- ✅ Improved Rust cache configuration
- ✅ Enhanced error handling and retry logic

### Key Files Modified
- `Dockerfile`: Updated base image digest
- `.github/workflows/ci.yml`: Removed inline audit ignores
- `.github/actions/setup-rust/action.yml`: Fixed deprecated cache config
- `.github/workflows/maintenance.yml`: Updated Python action version
- `.github/actions/security-scan/action.yml`: Updated Trivy version
