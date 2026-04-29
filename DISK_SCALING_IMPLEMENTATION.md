# Proactive Disk Scaling Implementation Summary

## Issue #578: Implement Proactive Disk Scaling using EBS/local-path provisioners

### Overview
Implemented automatic PVC expansion to prevent 'Disk Full' outages for Stellar validator nodes as the ledger database grows.

### Implementation Details

#### 1. Core Module: `src/controller/disk_scaler.rs`
New module implementing the disk scaling logic:

**Key Functions:**
- `get_disk_usage()`: Monitors disk usage by executing `df` command in pods
- `check_and_expand()`: Main orchestration function that checks usage and triggers expansion
- `expand_pvc()`: Patches PVC with new size
- `supports_expansion()`: Validates storage class supports volume expansion
- `calculate_new_size()`: Computes new PVC size based on increment percentage

**Key Types:**
- `DiskScalerConfig`: Configuration for scaling behavior
- `DiskUsage`: Disk usage information (capacity, used, percentage)
- `ScalingResult`: Enum representing expansion outcomes

**Features:**
- Threshold-based expansion (default: 80% usage)
- Configurable expansion increment (default: 50%)
- Rate limiting (default: 1 hour between expansions)
- Safety limits (default: max 10 expansions per PVC)
- Annotation tracking for expansion count and timestamp

#### 2. Metrics Integration: `src/controller/metrics.rs`
Added four new Prometheus metrics:

```rust
stellar_pvc_disk_usage_percent    // Current disk usage (0-100)
stellar_pvc_expansion_total       // Counter of expansion events
stellar_pvc_size_bytes            // Current PVC size in bytes
stellar_pvc_expansion_count       // Number of expansions performed
```

Helper functions:
- `set_pvc_disk_usage_percent()`
- `increment_pvc_expansion_total()`
- `set_pvc_size_bytes()`
- `set_pvc_expansion_count()`

#### 3. Configuration: `src/controller/operator_config.rs`
Added `DiskScalingConfig` struct with fields:
- `enabled`: Enable/disable automatic scaling
- `expansion_threshold`: Usage percentage trigger (0-100)
- `expansion_increment`: Size increase percentage
- `min_expansion_interval_secs`: Rate limiting
- `max_expansions`: Safety limit

Configuration loaded from operator ConfigMap at `/etc/stellar-operator/config.yaml`

#### 4. Reconciliation Integration: `src/controller/reconciler.rs`
Added disk scaling check in reconciliation loop (step 10d):
- Runs after health checks and before OCI snapshots
- Only executes when not in dry-run mode
- Emits Kubernetes events for expansion outcomes
- Updates Prometheus metrics
- Handles all scaling result types (success, rate-limited, max reached, failed)

#### 5. Documentation: `docs/proactive-disk-scaling.md`
Comprehensive documentation covering:
- Feature overview and benefits
- Supported storage providers (AWS EBS, GCP PD, Azure Disks)
- Configuration parameters
- How it works (step-by-step flow)
- Kubernetes events emitted
- Prometheus metrics and alerting examples
- Cost management and tracking
- Troubleshooting guide
- Best practices
- Storage class configuration requirements

#### 6. Example Configuration: `config/samples/disk-scaling-example.yaml`
Complete example including:
- StellarNode CR with storage configuration
- StorageClass with `allowVolumeExpansion: true`
- PrometheusRule with 4 alerts:
  - `StellarHighDiskUsage`: Warning at 75% usage
  - `StellarMaxExpansionsReached`: Critical when limit reached
  - `StellarDiskExpansionStalled`: Critical when expansion fails
  - `StellarRapidDiskGrowth`: Warning for abnormal growth

#### 7. Operator Configuration: `config/operator-config.yaml`
Added `diskScaling` section with all configuration parameters and defaults.

#### 8. Tests: `src/controller/disk_scaler_test.rs`
Comprehensive unit tests covering:
- Disk usage percentage calculation
- Quantity parsing (Gi, Ti, Mi, etc.)
- Size formatting
- New size calculation
- df output parsing
- Multiple expansion scenarios
- Edge cases and error handling

#### 9. Module Integration: `src/controller/mod.rs`
- Added `disk_scaler` module declaration
- Added `disk_scaler_test` test module
- Exported public types and functions

#### 10. README Update: `README.md`
Added disk scaling to key features list.

### Acceptance Criteria ✅

✅ **Monitor disk usage percentage on managed PVCs**
- Implemented via `get_disk_usage()` function
- Executes `df` command in pods to get real-time usage
- Calculates usage percentage from capacity and used bytes

✅ **Automatically trigger expand-pvc when usage exceeds 80%**
- Configurable threshold (default: 80%)
- Integrated into reconciliation loop
- Checks threshold on every reconciliation cycle

✅ **Coordinate with the storage provider's expansion limits**
- Validates storage class supports expansion via `supports_expansion()`
- Checks `allowVolumeExpansion` field on StorageClass
- Respects cloud provider capabilities (EBS, GCP PD, Azure Disks)

✅ **Log every expansion event for cost auditing**
- Emits Kubernetes events with expansion details
- Tracks expansion count in PVC annotations
- Records timestamp of each expansion
- Exports Prometheus metrics for monitoring
- All events queryable via `kubectl get events`

### Storage Provider Support

| Provider | Status | Notes |
|----------|--------|-------|
| AWS EBS | ✅ Supported | Online expansion, no pod restart |
| GCP Persistent Disks | ✅ Supported | Online expansion, no pod restart |
| Azure Disks | ✅ Supported | Online expansion, no pod restart |
| Local Storage | ⚠️ Not Supported | Requires manual intervention |

### Configuration Example

```yaml
diskScaling:
  enabled: true
  expansionThreshold: 80        # Trigger at 80% usage
  expansionIncrement: 50        # Increase by 50%
  minExpansionIntervalSecs: 3600  # 1 hour between expansions
  maxExpansions: 10             # Maximum 10 expansions per PVC
```

### Expansion Flow Example

1. **Initial State**: 100Gi PVC, 85Gi used (85% usage)
2. **Threshold Check**: 85% > 80% threshold → trigger expansion
3. **Rate Limit Check**: No expansion in last hour → proceed
4. **Safety Check**: Expansion count < 10 → proceed
5. **Storage Class Check**: `allowVolumeExpansion: true` → proceed
6. **Calculate New Size**: 100Gi + 50% = 150Gi
7. **Patch PVC**: Update PVC spec with new size
8. **Update Annotations**: Increment count, record timestamp
9. **Emit Event**: `DiskExpanded` event with details
10. **Update Metrics**: Increment counter, update gauges

### Cost Tracking

Every expansion is auditable via:
- Kubernetes events: `kubectl get events --field-selector reason=DiskExpanded`
- PVC annotations: `stellar.org/disk-expansion-count`, `stellar.org/last-disk-expansion`
- Prometheus metrics: `stellar_pvc_expansion_total`, `stellar_pvc_size_bytes`

### Monitoring & Alerting

Four Prometheus alerts included:
1. **HighDiskUsage**: Warning at 75% (before expansion threshold)
2. **MaxExpansionsReached**: Critical when safety limit hit
3. **DiskExpansionStalled**: Critical when expansion fails
4. **RapidDiskGrowth**: Warning for abnormal growth patterns

### Safety Features

1. **Rate Limiting**: Prevents rapid successive expansions (default: 1 hour)
2. **Max Expansions**: Safety limit to prevent runaway costs (default: 10)
3. **Storage Class Validation**: Only expands if provider supports it
4. **Dry-Run Support**: Can be tested without actual expansion
5. **Event Logging**: All actions logged for audit trail

### Testing

Comprehensive test coverage:
- Unit tests for all calculation functions
- df output parsing tests
- Quantity conversion tests (Gi, Ti, Mi)
- Multiple expansion scenario tests
- Edge case handling

### Files Modified/Created

**New Files:**
- `src/controller/disk_scaler.rs` (main implementation)
- `src/controller/disk_scaler_test.rs` (tests)
- `docs/proactive-disk-scaling.md` (documentation)
- `config/samples/disk-scaling-example.yaml` (example)
- `DISK_SCALING_IMPLEMENTATION.md` (this file)

**Modified Files:**
- `src/controller/mod.rs` (module integration)
- `src/controller/metrics.rs` (metrics addition)
- `src/controller/operator_config.rs` (configuration)
- `src/controller/reconciler.rs` (reconciliation integration)
- `config/operator-config.yaml` (config example)
- `README.md` (feature list)

### Next Steps

1. **Testing**: Run integration tests with actual Kubernetes cluster
2. **Validation**: Test with different storage providers (EBS, GCP PD, Azure)
3. **Monitoring**: Verify Prometheus metrics are exported correctly
4. **Documentation Review**: Ensure all docs are accurate and complete
5. **Performance**: Monitor reconciliation loop performance impact
6. **Cost Analysis**: Track actual expansion costs in production

### Known Limitations

1. **Local Storage**: Cannot be expanded automatically (Kubernetes limitation)
2. **Volume Shrinking**: Not supported (Kubernetes limitation)
3. **Pod Exec Required**: Needs to execute `df` command in pods
4. **Filesystem Resize**: Some filesystems may require manual resize (rare)
5. **Provider Limits**: Each cloud provider has maximum volume size limits

### Future Enhancements

1. **Predictive Scaling**: Use historical growth rate to predict future needs
2. **Cost Optimization**: Suggest optimal expansion increment based on growth patterns
3. **Multi-PVC Support**: Handle nodes with multiple PVCs
4. **Custom Metrics**: Support custom disk usage metrics from node exporters
5. **Webhook Integration**: Notify external systems of expansion events
