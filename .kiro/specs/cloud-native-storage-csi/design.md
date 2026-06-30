# Design Document

## System Overview

The Cloud-Native Storage Management system extends Stellar-K8s with comprehensive CSI integration, automated volume lifecycle management, and intelligent storage optimization. The system consists of four main components:

1. **Volume Lifecycle Controller**: Manages volume provisioning, expansion, and deletion
2. **Snapshot Controller**: Handles scheduled snapshots and retention policies
3. **Storage Analytics Engine**: Provides usage tracking, forecasting, and cost optimization
4. **Storage Tier Manager**: Automates data movement between hot/warm/cold storage tiers

## Architecture

### Component Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                    Stellar-K8s Operator                          │
│                                                                   │
│  ┌──────────────────┐  ┌──────────────────┐  ┌───────────────┐ │
│  │  Volume          │  │  Snapshot        │  │  Storage      │ │
│  │  Lifecycle       │  │  Controller      │  │  Analytics    │ │
│  │  Controller      │  │                  │  │  Engine       │ │
│  └────────┬─────────┘  └────────┬─────────┘  └───────┬───────┘ │
│           │                     │                     │          │
│           │                     │                     │          │
└───────────┼─────────────────────┼─────────────────────┼──────────┘
            │                     │                     │
            ▼                     ▼                     ▼
┌───────────────────────────────────────────────────────────────────┐
│                    Kubernetes API Server                          │
│                                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ StorageClass │  │ VolumeSnapshot│  │  PersistentVolume    │  │
│  │     API      │  │      API      │  │       Claims         │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
└─────────┼──────────────────┼─────────────────────┼──────────────┘
          │                  │                     │
          ▼                  ▼                     ▼
┌─────────────────────────────────────────────────────────────────┐
│                      CSI Drivers                                 │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐ │
│  │  AWS EBS     │  │   GCP PD     │  │    Azure Disk        │ │
│  │  CSI Driver  │  │  CSI Driver  │  │    CSI Driver        │ │
│  └──────────────┘  └──────────────┘  └──────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```


### Data Flow

1. **Volume Provisioning**:
   - Operator watches StellarNode CRD creation
   - Volume Lifecycle Controller creates PVC with appropriate StellarStorageClass
   - CSI driver provisions volume with specified performance characteristics
   - Storage Analytics Engine begins tracking metrics

2. **Auto-Expansion**:
   - Volume Lifecycle Controller monitors PVC usage every 60s
   - When threshold exceeded, controller updates PVC spec
   - CSI driver expands volume without downtime
   - Analytics Engine updates capacity forecasts

3. **Snapshot Management**:
   - Snapshot Controller runs cron scheduler
   - Creates VolumeSnapshot via CSI driver
   - Enforces retention policies (count + age)
   - Deletes expired snapshots

4. **Tier Migration**:
   - Storage Tier Manager analyzes I/O patterns
   - Identifies volumes for tier change
   - Clones volume to new storage class
   - Updates PVC binding and deletes old volume

## CRD Schema

### StellarStorageClass

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarStorageClass
metadata:
  name: validator-high-performance
spec:
  provider: AWS  # AWS | GCP | Azure
  storageType: SSD  # SSD | HDD | NVMe
  performanceProfile: Validator  # Validator | Horizon | Archive | Custom
  tieringPolicy: Hot  # Hot | Warm | Cold | Auto
  
  volumeExpansion:
    enabled: true
    autoExpand: true
    thresholdPercent: 80
    incrementGiB: 50
  
  snapshotPolicy:
    enabled: true
    schedule: "0 2 * * *"  # Daily at 2 AM
    retentionDays: 30
    retentionCount: 10
  
  performanceTuning:
    iops: 10000
    throughputMBps: 500
    latencyTarget: Low  # Low | Medium | High
  
  encryption:
    enabled: true
    kmsKeyId: "arn:aws:kms:us-east-1:123456789012:key/12345678-1234-1234-1234-123456789012"
    encryptionAlgorithm: "AES-256"

status:
  state: Active  # Pending | Active | Failed
  usageStatistics:
    totalVolumeCount: 5
    totalSizeGiB: 500
    totalCostEstimated: 75.50
  lastReconciled: "2026-06-02T10:30:00Z"
```


## Controller Implementation

### Volume Lifecycle Controller

**Responsibilities:**
- Watch StellarStorageClass and PVC resources
- Create corresponding Kubernetes StorageClass with CSI parameters
- Monitor volume usage and trigger auto-expansion
- Manage volume deletion according to reclaim policy

**Reconciliation Logic:**

```rust
async fn reconcile_storage_class(ctx: Arc<Context>, ssc: Arc<StellarStorageClass>) -> Result<Action> {
    let client = ctx.client.clone();
    
    // 1. Validate CSI driver availability
    if !is_csi_driver_available(&client, &ssc.spec.provider).await? {
        return Err(Error::CSIDriverNotFound(ssc.spec.provider.clone()));
    }
    
    // 2. Create or update Kubernetes StorageClass
    let storage_class = build_storage_class(&ssc)?;
    apply_storage_class(&client, storage_class).await?;
    
    // 3. Update usage statistics
    let stats = calculate_usage_statistics(&client, &ssc).await?;
    update_status(&client, &ssc, stats).await?;
    
    Ok(Action::requeue(Duration::from_secs(300)))
}

async fn monitor_volume_usage(ctx: Arc<Context>) -> Result<()> {
    loop {
        let pvcs = list_managed_pvcs(&ctx.client).await?;
        
        for pvc in pvcs {
            let usage_percent = get_pvc_usage_percent(&pvc).await?;
            let ssc = get_stellar_storage_class(&ctx.client, &pvc).await?;
            
            if ssc.spec.volume_expansion.auto_expand && 
               usage_percent > ssc.spec.volume_expansion.threshold_percent {
                expand_volume(&ctx.client, &pvc, &ssc).await?;
            }
        }
        
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
```


### Snapshot Controller

**Responsibilities:**
- Execute scheduled snapshot creation via cron
- Enforce retention policies (count and age-based)
- Clean up expired snapshots
- Handle snapshot failures with retry logic

**Implementation:**

```rust
struct SnapshotController {
    client: Client,
    scheduler: CronScheduler,
}

impl SnapshotController {
    async fn reconcile_snapshot_policy(&self, ssc: &StellarStorageClass) -> Result<Action> {
        if !ssc.spec.snapshot_policy.enabled {
            return Ok(Action::await_change());
        }
        
        // Schedule snapshot jobs
        let schedule = cron::Schedule::from_str(&ssc.spec.snapshot_policy.schedule)?;
        self.scheduler.add_job(schedule, move || {
            self.create_snapshots_for_storage_class(ssc).await
        });
        
        // Enforce retention policies
        self.enforce_retention_policy(ssc).await?;
        
        Ok(Action::requeue(Duration::from_secs(3600)))
    }
    
    async fn create_snapshots_for_storage_class(&self, ssc: &StellarStorageClass) -> Result<()> {
        let pvcs = self.list_pvcs_for_storage_class(ssc).await?;
        
        for pvc in pvcs {
            let snapshot = VolumeSnapshot {
                metadata: ObjectMeta {
                    name: Some(format!("{}-{}", pvc.metadata.name.unwrap(), chrono::Utc::now().timestamp())),
                    labels: Some(btreemap! {
                        "stellar.org/snapshot-policy".to_string() => "scheduled".to_string(),
                        "stellar.org/source-pvc".to_string() => pvc.metadata.name.unwrap(),
                    }),
                    ..Default::default()
                },
                spec: VolumeSnapshotSpec {
                    source: VolumeSnapshotSource {
                        persistent_volume_claim_name: Some(pvc.metadata.name.unwrap()),
                        ..Default::default()
                    },
                    volume_snapshot_class_name: Some("csi-snapshot-class".to_string()),
                    ..Default::default()
                },
            };
            
            self.client.create(&snapshot).await?;
        }
        
        Ok(())
    }
    
    async fn enforce_retention_policy(&self, ssc: &StellarStorageClass) -> Result<()> {
        let snapshots = self.list_snapshots_for_storage_class(ssc).await?;
        let policy = &ssc.spec.snapshot_policy;
        
        // Group by source PVC
        let mut snapshots_by_pvc: HashMap<String, Vec<VolumeSnapshot>> = HashMap::new();
        for snapshot in snapshots {
            let pvc_name = snapshot.metadata.labels
                .get("stellar.org/source-pvc")
                .cloned()
                .unwrap_or_default();
            snapshots_by_pvc.entry(pvc_name).or_default().push(snapshot);
        }
        
        for (_, mut pvc_snapshots) in snapshots_by_pvc {
            // Sort by creation time (newest first)
            pvc_snapshots.sort_by_key(|s| s.metadata.creation_timestamp);
            pvc_snapshots.reverse();
            
            // Delete snapshots exceeding retention count
            if pvc_snapshots.len() > policy.retention_count as usize {
                for snapshot in pvc_snapshots.iter().skip(policy.retention_count as usize) {
                    self.delete_snapshot(snapshot).await?;
                }
            }
            
            // Delete snapshots older than retention days
            let cutoff_time = chrono::Utc::now() - chrono::Duration::days(policy.retention_days);
            for snapshot in pvc_snapshots {
                if let Some(created) = snapshot.metadata.creation_timestamp {
                    if created < cutoff_time {
                        self.delete_snapshot(&snapshot).await?;
                    }
                }
            }
        }
        
        Ok(())
    }
}
```


### Storage Analytics Engine

**Responsibilities:**
- Collect storage usage metrics from Prometheus
- Calculate growth rates and capacity forecasts
- Generate cost optimization recommendations
- Track storage costs across providers

**Implementation:**

```rust
struct StorageAnalyticsEngine {
    client: Client,
    prometheus_client: PrometheusClient,
    pricing_config: PricingConfig,
}

impl StorageAnalyticsEngine {
    async fn analyze_storage_usage(&self) -> Result<StorageAnalytics> {
        let pvcs = self.list_all_managed_pvcs().await?;
        let mut analytics = StorageAnalytics::default();
        
        for pvc in pvcs {
            let usage = self.get_pvc_usage_metrics(&pvc).await?;
            let growth_rate = self.calculate_growth_rate(&pvc, Duration::from_days(7)).await?;
            let forecast = self.forecast_capacity(&pvc, growth_rate).await?;
            
            analytics.total_size_gib += usage.size_gib;
            analytics.growth_rate_gib_per_day += growth_rate;
            
            // Check if capacity will be reached
            if let Some(days_until_full) = forecast.days_until_90_percent {
                if days_until_full < 30 {
                    analytics.capacity_warnings.push(CapacityWarning {
                        pvc_name: pvc.metadata.name.unwrap(),
                        days_until_full,
                        current_usage_percent: usage.usage_percent,
                    });
                }
            }
        }
        
        analytics.total_cost_usd = self.calculate_total_cost(&pvcs).await?;
        analytics.optimization_recommendations = self.generate_recommendations(&pvcs).await?;
        
        Ok(analytics)
    }
    
    async fn calculate_growth_rate(&self, pvc: &PersistentVolumeClaim, window: Duration) -> Result<f64> {
        let query = format!(
            "rate(kubelet_volume_stats_used_bytes{{persistentvolumeclaim=\"{}\"}}[{}d])",
            pvc.metadata.name.unwrap(),
            window.num_days()
        );
        
        let result = self.prometheus_client.query(&query).await?;
        let bytes_per_second = result.as_f64()?;
        
        // Convert to GiB per day
        Ok(bytes_per_second * 86400.0 / (1024.0 * 1024.0 * 1024.0))
    }
    
    async fn forecast_capacity(&self, pvc: &PersistentVolumeClaim, growth_rate: f64) -> Result<CapacityForecast> {
        let current_usage = self.get_pvc_usage_gib(pvc).await?;
        let total_capacity = pvc.spec.resources.requests.get("storage")
            .and_then(|q| q.parse::<f64>().ok())
            .unwrap_or(0.0);
        
        let capacity_90_percent = total_capacity * 0.9;
        let remaining_capacity = capacity_90_percent - current_usage;
        
        let days_until_90_percent = if growth_rate > 0.0 {
            Some((remaining_capacity / growth_rate).ceil() as u32)
        } else {
            None
        };
        
        Ok(CapacityForecast {
            current_usage_gib: current_usage,
            forecast_30_days: current_usage + (growth_rate * 30.0),
            forecast_60_days: current_usage + (growth_rate * 60.0),
            forecast_90_days: current_usage + (growth_rate * 90.0),
            days_until_90_percent,
        })
    }
}
```


### Cost Optimization

**Recommendation Engine:**

```rust
impl StorageAnalyticsEngine {
    async fn generate_recommendations(&self, pvcs: &[PersistentVolumeClaim]) -> Result<Vec<Recommendation>> {
        let mut recommendations = Vec::new();
        
        for pvc in pvcs {
            let ssc = self.get_stellar_storage_class(pvc).await?;
            let metrics = self.get_pvc_metrics(pvc).await?;
            
            // Check for overprovisioned IOPS
            if let Some(provisioned_iops) = ssc.spec.performance_tuning.iops {
                let actual_iops = metrics.avg_iops_7d;
                let utilization = actual_iops / provisioned_iops as f64;
                
                if utilization < 0.5 {
                    let optimized_iops = (actual_iops * 1.5).ceil() as u32;
                    let monthly_savings = self.calculate_iops_cost_diff(
                        &ssc.spec.provider,
                        provisioned_iops,
                        optimized_iops
                    )?;
                    
                    recommendations.push(Recommendation {
                        id: format!("iops-{}", pvc.metadata.name.unwrap()),
                        severity: if monthly_savings > 500.0 { "High" } else { "Medium" },
                        affected_volume: pvc.metadata.name.unwrap(),
                        current_cost_usd_monthly: self.calculate_iops_cost(&ssc.spec.provider, provisioned_iops)?,
                        optimized_cost_usd_monthly: self.calculate_iops_cost(&ssc.spec.provider, optimized_iops)?,
                        savings_usd_monthly: monthly_savings,
                        action_required: format!(
                            "Reduce provisioned IOPS from {} to {} ({}% utilization)",
                            provisioned_iops, optimized_iops, (utilization * 100.0) as u32
                        ),
                    });
                }
            }
            
            // Check for unused volumes
            if metrics.io_operations_30d == 0 && !is_volume_attached(pvc).await? {
                recommendations.push(Recommendation {
                    id: format!("unused-{}", pvc.metadata.name.unwrap()),
                    severity: "Medium",
                    affected_volume: pvc.metadata.name.unwrap(),
                    current_cost_usd_monthly: self.calculate_volume_cost(pvc, &ssc).await?,
                    optimized_cost_usd_monthly: 0.0,
                    savings_usd_monthly: self.calculate_volume_cost(pvc, &ssc).await?,
                    action_required: "Delete unused volume (zero I/O for 30+ days)".to_string(),
                });
            }
            
            // Check for tier migration opportunities
            if ssc.spec.tiering_policy == TieringPolicy::Hot && metrics.avg_iops_30d < 100.0 {
                let current_cost = self.calculate_volume_cost(pvc, &ssc).await?;
                let warm_cost = self.calculate_tier_cost(pvc, Tier::Warm).await?;
                
                recommendations.push(Recommendation {
                    id: format!("tier-{}", pvc.metadata.name.unwrap()),
                    severity: "Low",
                    affected_volume: pvc.metadata.name.unwrap(),
                    current_cost_usd_monthly: current_cost,
                    optimized_cost_usd_monthly: warm_cost,
                    savings_usd_monthly: current_cost - warm_cost,
                    action_required: format!(
                        "Migrate from Hot to Warm tier (low I/O: {} IOPS avg)",
                        metrics.avg_iops_30d as u32
                    ),
                });
            }
        }
        
        // Sort by savings (descending)
        recommendations.sort_by(|a, b| b.savings_usd_monthly.partial_cmp(&a.savings_usd_monthly).unwrap());
        
        Ok(recommendations)
    }
}
```


## CSI Driver Integration

### AWS EBS CSI Driver

**StorageClass Parameters:**

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-validator-aws
provisioner: ebs.csi.aws.com
parameters:
  type: io2  # gp3, io2, st1
  iops: "10000"
  throughput: "500"  # MB/s (gp3 only)
  encrypted: "true"
  kmsKeyId: "arn:aws:kms:us-east-1:123456789012:key/..."
allowVolumeExpansion: true
volumeBindingMode: WaitForFirstConsumer
```

**Performance Profiles:**

| Profile    | Type | IOPS  | Throughput | Cost/GB/Month |
|------------|------|-------|------------|---------------|
| Validator  | io2  | 10000 | 500 MB/s   | $0.125        |
| Horizon    | gp3  | 6000  | 250 MB/s   | $0.080        |
| Archive    | st1  | -     | 500 MB/s   | $0.045        |

### GCP Persistent Disk CSI Driver

**StorageClass Parameters:**

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-validator-gcp
provisioner: pd.csi.storage.gke.io
parameters:
  type: pd-extreme  # pd-ssd, pd-balanced, pd-extreme, pd-standard
  provisioned-iops-on-create: "50000"
  replication-type: regional-pd  # none, regional-pd
allowVolumeExpansion: true
volumeBindingMode: WaitForFirstConsumer
```

**Performance Profiles:**

| Profile    | Type        | IOPS  | Throughput | Cost/GB/Month |
|------------|-------------|-------|------------|---------------|
| Validator  | pd-extreme  | 50000 | 1200 MB/s  | $0.125        |
| Horizon    | pd-balanced | 6000  | 240 MB/s   | $0.100        |
| Archive    | pd-standard | 1500  | 240 MB/s   | $0.040        |

### Azure Disk CSI Driver

**StorageClass Parameters:**

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-validator-azure
provisioner: disk.csi.azure.com
parameters:
  skuName: UltraSSD_LRS  # Premium_LRS, StandardSSD_LRS, UltraSSD_LRS
  diskIOPSReadWrite: "20000"
  diskMBpsReadWrite: "500"
  cachingMode: None
allowVolumeExpansion: true
volumeBindingMode: WaitForFirstConsumer
```

**Performance Profiles:**

| Profile    | SKU            | IOPS  | Throughput | Cost/GB/Month |
|------------|----------------|-------|------------|---------------|
| Validator  | UltraSSD_LRS   | 20000 | 500 MB/s   | $0.122        |
| Horizon    | Premium_LRS    | 5000  | 200 MB/s   | $0.135        |
| Archive    | StandardSSD_LRS| 500   | 60 MB/s    | $0.075        |


## Prometheus Metrics

### Volume Lifecycle Metrics

```
# Volume expansion metrics
stellar_storage_volume_expansion_total{node_name, storage_class, provider} counter
stellar_storage_volume_expansion_errors_total{node_name, storage_class, error_type} counter
stellar_storage_volume_usage_percent{node_name, pvc_name, storage_class} gauge

# Volume provisioning metrics
stellar_storage_volume_provisioning_duration_seconds{provider, storage_type} histogram
stellar_storage_volume_provisioning_errors_total{provider, error_type} counter

# Storage capacity metrics
stellar_storage_total_size_gib{storage_class, provider} gauge
stellar_storage_total_cost_usd_monthly{storage_class, provider} gauge
```

### Snapshot Metrics

```
# Snapshot creation/deletion metrics
stellar_storage_snapshot_created_total{node_name, storage_class} counter
stellar_storage_snapshot_deleted_total{node_name, storage_class, reason} counter
stellar_storage_snapshot_failed_total{node_name, storage_class, error_type} counter

# Snapshot age and size metrics
stellar_storage_snapshot_age_seconds{snapshot_name, source_pvc} gauge
stellar_storage_snapshot_size_gib{snapshot_name, source_pvc} gauge
stellar_storage_snapshot_count{node_name, storage_class} gauge
```

### Performance Metrics

```
# IOPS and throughput metrics
stellar_storage_iops_configured{pvc_name, storage_class, provider} gauge
stellar_storage_iops_actual{pvc_name, storage_class} gauge
stellar_storage_iops_utilization_percent{pvc_name, storage_class} gauge

stellar_storage_throughput_mbps_configured{pvc_name, storage_class, provider} gauge
stellar_storage_throughput_mbps_actual{pvc_name, storage_class} gauge

stellar_storage_latency_milliseconds{pvc_name, operation, percentile} histogram
```

### Analytics Metrics

```
# Growth and forecasting metrics
stellar_storage_growth_rate_gib_per_day{pvc_name, node_name} gauge
stellar_storage_forecast_gib{pvc_name, days} gauge
stellar_storage_days_until_full{pvc_name} gauge

# Cost optimization metrics
stellar_storage_unused_iops_percent{pvc_name} gauge
stellar_storage_cost_optimization_potential_usd_monthly{storage_class} gauge
stellar_storage_recommendation_count{severity} gauge
```

### Tier Management Metrics

```
# Tiering metrics
stellar_storage_tier_migrations_total{from_tier, to_tier, reason} counter
stellar_storage_volumes_by_tier{tier, storage_class} gauge
stellar_storage_tier_cost_savings_usd_monthly{from_tier, to_tier} gauge
stellar_storage_tier_io_operations{pvc_name, tier, operation} counter
```


## REST API Endpoints

### Storage Management API

```
GET  /api/v1/storage/classes
     - List all StellarStorageClass resources
     - Response: {storageClasses: [...], total: 10}

GET  /api/v1/storage/classes/{name}
     - Get specific StellarStorageClass details
     - Response: {name, spec, status}

POST /api/v1/storage/classes
     - Create new StellarStorageClass
     - Request: {name, spec}

GET  /api/v1/storage/volumes
     - List all managed PVCs with metrics
     - Query params: storage_class, node_name, provider
     - Response: {volumes: [...], total: 50}

GET  /api/v1/storage/volumes/{name}
     - Get volume details with usage metrics
     - Response: {name, size, usage_percent, iops, throughput}

POST /api/v1/storage/volumes/{name}/expand
     - Manually trigger volume expansion
     - Request: {new_size_gib: 150}

GET  /api/v1/storage/snapshots
     - List all snapshots
     - Query params: pvc_name, storage_class
     - Response: {snapshots: [...], total: 100}

POST /api/v1/storage/snapshots
     - Create manual snapshot
     - Request: {pvc_name, snapshot_class}

DELETE /api/v1/storage/snapshots/{name}
     - Delete specific snapshot

POST /api/v1/storage/clone
     - Clone volume from snapshot or PVC
     - Request: {source_type, source_name, target_name}

GET  /api/v1/storage/analytics
     - Get storage analytics summary
     - Response: {total_size, total_cost, growth_rate, forecasts}

GET  /api/v1/storage/recommendations
     - Get cost optimization recommendations
     - Response: {recommendations: [...], total_savings: 1250.50}

POST /api/v1/storage/recommendations/{id}/apply
     - Apply specific recommendation
     - Response: {status, applied_changes}
```


## Grafana Dashboard

### Panel Layout

**Row 1: Overview Metrics**
- Total Storage Capacity (gauge)
- Total Storage Used (gauge with percentage)
- Monthly Cost (stat with trend)
- Storage Growth Rate (stat)

**Row 2: Volume Distribution**
- Volumes by Tier (pie chart: Hot/Warm/Cold)
- Volumes by Provider (pie chart: AWS/GCP/Azure)
- Volumes by Performance Profile (bar chart)
- Volume Count by Storage Class (table)

**Row 3: Performance Monitoring**
- IOPS Utilization Heatmap (by PVC)
- Throughput Utilization Heatmap (by PVC)
- Storage Latency p95/p99 (time series)
- Top 5 Volumes by IOPS (table)

**Row 4: Capacity and Forecasting**
- Storage Usage Over Time (stacked area chart)
- Capacity Forecast 30/60/90 Days (line chart with projections)
- Volumes Nearing Capacity (alert list: >80%)
- Auto-Expansion Events Timeline

**Row 5: Snapshot Management**
- Snapshot Count by Storage Class (bar chart)
- Snapshot Total Size (stat)
- Snapshot Creation/Deletion Rate (time series)
- Failed Snapshots (alert list)

**Row 6: Cost Analysis**
- Cost by Storage Class (stacked bar chart)
- Cost by Provider (pie chart)
- Cost Trend Over Time (area chart)
- Cost Optimization Savings Potential (stat)

**Row 7: Recommendations**
- Active Recommendations (table with severity, savings, action)
- Tier Migration Events (timeline)
- Unused Volumes (alert list)
- Overprovisioned IOPS (alert list)

**Row 8: System Health**
- Volume Lifecycle Controller Status (indicator)
- Snapshot Controller Status (indicator)
- Analytics Engine Status (indicator)
- CSI Driver Health by Provider (table)


## Security Considerations

### Encryption at Rest

1. **KMS Integration**: All storage classes support encryption with cloud provider KMS
2. **Key Rotation**: Automated key rotation policy for encrypted volumes
3. **Secret Management**: KMS key IDs stored in Kubernetes Secrets
4. **Encryption Verification**: Controller verifies encryption status on reconciliation

### Access Control

1. **RBAC**: Separate roles for storage-admin, storage-viewer, snapshot-manager
2. **ServiceAccount**: Dedicated ServiceAccount for each controller with minimal permissions
3. **NetworkPolicy**: Restrict API access to authorized pods only
4. **Audit Logging**: All storage operations logged for compliance

### Snapshot Security

1. **Snapshot Encryption**: Snapshots inherit encryption from source volume
2. **Access Control**: Snapshots can only be cloned by authorized ServiceAccounts
3. **Retention Enforcement**: Automatic deletion prevents unauthorized data retention
4. **Cross-Account Prevention**: Snapshots cannot be shared across cloud accounts without explicit policy

## Performance Benchmarks

### Volume Provisioning Time

| Provider | Storage Type | Size    | Provisioning Time |
|----------|--------------|---------|-------------------|
| AWS      | gp3          | 100 GiB | 3-5 seconds       |
| AWS      | io2          | 100 GiB | 5-8 seconds       |
| GCP      | pd-balanced  | 100 GiB | 4-6 seconds       |
| GCP      | pd-extreme   | 100 GiB | 6-10 seconds      |
| Azure    | Premium_LRS  | 100 GiB | 5-8 seconds       |
| Azure    | UltraSSD_LRS | 100 GiB | 8-12 seconds      |

### Volume Expansion Time

| Provider | Size Change  | Expansion Time | Downtime |
|----------|--------------|----------------|----------|
| AWS      | 100→150 GiB  | 10-15 seconds  | None     |
| GCP      | 100→150 GiB  | 15-20 seconds  | None     |
| Azure    | 100→150 GiB  | 20-30 seconds  | None     |

### Snapshot Creation Time

| Provider | Volume Size | Snapshot Time |
|----------|-------------|---------------|
| AWS      | 100 GiB     | 2-3 minutes   |
| GCP      | 100 GiB     | 1-2 minutes   |
| Azure    | 100 GiB     | 3-5 minutes   |

### Volume Clone Time

| Provider | Volume Size | Clone Time    | Method        |
|----------|-------------|---------------|---------------|
| AWS      | 100 GiB     | 30-45 seconds | Fast Snapshot |
| GCP      | 100 GiB     | 20-30 seconds | Instant Clone |
| Azure    | 100 GiB     | 40-60 seconds | Incremental   |


## Implementation Phases

### Phase 1: Core Infrastructure (Week 1-2)

**Deliverables:**
- StellarStorageClass CRD definition
- Volume Lifecycle Controller basic implementation
- CSI driver integration for AWS EBS
- Basic Prometheus metrics
- Unit tests for CRD validation

**Success Criteria:**
- StellarStorageClass can be created and reconciled
- Volumes provisioned with correct CSI parameters
- Basic metrics exposed for volume creation

### Phase 2: Multi-Cloud Support (Week 3-4)

**Deliverables:**
- GCP Persistent Disk CSI integration
- Azure Disk CSI integration
- Performance profile mapping for all providers
- Provider-specific parameter validation
- Integration tests for each provider

**Success Criteria:**
- All three providers working with correct parameters
- Performance profiles correctly mapped per provider
- Cross-provider parity for common features

### Phase 3: Volume Lifecycle Management (Week 5-6)

**Deliverables:**
- Auto-expansion monitoring and implementation
- Volume usage metrics collection
- Expansion event handling and retry logic
- Volume deletion with reclaim policy support
- E2E tests for volume lifecycle

**Success Criteria:**
- Volumes automatically expand when threshold exceeded
- Expansion completes without downtime
- Metrics accurately reflect usage and expansion events

### Phase 4: Snapshot Management (Week 7-8)

**Deliverables:**
- Snapshot Controller implementation
- Cron-based scheduling
- Retention policy enforcement (count + age)
- Snapshot creation/deletion with error handling
- Snapshot-related metrics and events

**Success Criteria:**
- Snapshots created according to schedule
- Old snapshots automatically deleted per policy
- Snapshot failures handled with retry logic

### Phase 5: Volume Cloning (Week 9)

**Deliverables:**
- Clone from snapshot functionality
- Clone from PVC functionality
- Clone performance optimization
- Clone-related metrics
- Documentation for cloning workflows

**Success Criteria:**
- Clones complete in <1 minute for 100GB volumes
- Clones properly labeled for tracking
- Cloning works across all providers


### Phase 6: Performance Optimization (Week 10-11)

**Deliverables:**
- Performance tuning implementation (IOPS/throughput)
- Performance metrics collection
- Latency monitoring integration
- Performance profile validation
- Performance benchmarking suite

**Success Criteria:**
- IOPS and throughput correctly configured per provider
- Performance metrics accurately reflect actual usage
- Validation prevents invalid performance configurations

### Phase 7: Storage Analytics (Week 12-13)

**Deliverables:**
- Storage Analytics Engine implementation
- Growth rate calculation
- Capacity forecasting (30/60/90 days)
- Cost calculation for all providers
- Analytics REST API endpoints

**Success Criteria:**
- Growth rates accurately calculated from historical data
- Forecasts predict capacity needs within 10% accuracy
- Cost estimates match actual cloud provider billing

### Phase 8: Multi-Tier Storage (Week 14-15)

**Deliverables:**
- Storage Tier Manager implementation
- I/O pattern analysis for tier recommendations
- Automated tier migration
- Tier-related metrics and events
- Tier migration testing

**Success Criteria:**
- Volumes automatically migrated based on I/O patterns
- Tier migrations complete without data loss
- Cost savings from tiering measurable

### Phase 9: Cost Optimization (Week 16-17)

**Deliverables:**
- Recommendation engine implementation
- IOPS utilization analysis
- Unused volume detection
- Snapshot optimization recommendations
- Recommendation REST API

**Success Criteria:**
- Recommendations identify 40% cost reduction opportunities
- Recommendations can be applied via API
- Applied recommendations result in measurable savings

### Phase 10: Dashboard and Documentation (Week 18-20)

**Deliverables:**
- Grafana dashboard with all panels
- Comprehensive documentation
- Best practices guide
- Troubleshooting guide
- Cost optimization playbook
- Example manifests for all use cases

**Success Criteria:**
- Dashboard displays all metrics in real-time
- Documentation covers all features comprehensively
- Users can successfully deploy without support


## Testing Strategy

### Unit Tests

1. **CRD Validation**: Test all StellarStorageClass field validations
2. **CSI Parameter Mapping**: Test correct parameter generation per provider
3. **Cost Calculations**: Test cost estimation formulas for all providers
4. **Growth Rate Calculations**: Test various growth patterns
5. **Retention Policy Logic**: Test snapshot retention enforcement

### Integration Tests

1. **Volume Provisioning**: Test volume creation across all providers
2. **Volume Expansion**: Test auto-expansion and manual expansion
3. **Snapshot Creation**: Test scheduled and manual snapshots
4. **Volume Cloning**: Test clone from snapshot and PVC
5. **Tier Migration**: Test migration between tiers
6. **Metrics Collection**: Verify metrics are correctly exported

### End-to-End Tests

1. **Full Lifecycle**: Deploy StellarNode → auto-expand → snapshot → clone → delete
2. **Multi-Provider**: Deploy same workload across AWS, GCP, Azure
3. **Cost Optimization**: Verify recommendations lead to actual savings
4. **Performance**: Benchmark IOPS/throughput meet specifications
5. **Disaster Recovery**: Snapshot → restore → verify data integrity

### Load Tests

1. **Concurrent Expansions**: 100 volumes expanding simultaneously
2. **Snapshot Scaling**: 1000+ snapshots with retention enforcement
3. **Clone Performance**: 50 concurrent clone operations
4. **Metrics Volume**: High-cardinality metrics with 500+ volumes
5. **API Performance**: REST API under 1000 req/s load

### Chaos Tests

1. **CSI Driver Failure**: Driver crashes during volume provisioning
2. **API Server Downtime**: Kubernetes API unavailable during reconciliation
3. **Storage Backend Failure**: Cloud provider API errors
4. **Partial Network Partition**: Controllers can't reach some nodes
5. **Resource Exhaustion**: OOM during analytics processing


## Dependencies

### External Crates

```toml
[dependencies]
# Existing dependencies from main Cargo.toml
kube = { version = "0.94", features = ["runtime", "derive"] }
k8s-openapi = { version = "0.26", features = ["v1_30"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# New dependencies for storage management
cron = "0.15"  # Already exists - for snapshot scheduling
prometheus-client = { version = "0.22", optional = true }  # Already exists - metrics
reqwest = { version = "0.12", features = ["json"] }  # Already exists - for CSI driver API
async-trait = "0.1"  # Already exists - for controller traits
```

### CSI Drivers (External)

**Must be pre-installed on cluster:**
- AWS EBS CSI Driver: `aws-ebs-csi-driver` >= v1.25
- GCP PD CSI Driver: `gcp-pd-csi-driver` >= v1.10
- Azure Disk CSI Driver: `azuredisk-csi-driver` >= v1.28
- Snapshot Controller: `snapshot-controller` >= v6.0

**Installation:**
```bash
# AWS EBS CSI Driver
kubectl apply -k "github.com/kubernetes-sigs/aws-ebs-csi-driver/deploy/kubernetes/overlays/stable/?ref=release-1.25"

# GCP PD CSI Driver (usually pre-installed on GKE)
kubectl apply -f https://github.com/kubernetes-sigs/gcp-compute-persistent-disk-csi-driver/raw/master/deploy/kubernetes/manifests/deploy-driver.yaml

# Azure Disk CSI Driver (usually pre-installed on AKS)
kubectl apply -f https://raw.githubusercontent.com/kubernetes-sigs/azuredisk-csi-driver/master/deploy/install-driver.sh

# Snapshot Controller (required for all providers)
kubectl apply -f https://raw.githubusercontent.com/kubernetes-csi/external-snapshotter/master/deploy/kubernetes/snapshot-controller/rbac-snapshot-controller.yaml
kubectl apply -f https://raw.githubusercontent.com/kubernetes-csi/external-snapshotter/master/deploy/kubernetes/snapshot-controller/setup-snapshot-controller.yaml
```

### Cloud Provider IAM Permissions

**AWS IAM Policy:**
```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "ec2:CreateVolume",
        "ec2:DeleteVolume",
        "ec2:ModifyVolume",
        "ec2:CreateSnapshot",
        "ec2:DeleteSnapshot",
        "ec2:DescribeVolumes",
        "ec2:DescribeSnapshots",
        "ec2:CreateTags",
        "kms:CreateGrant",
        "kms:Decrypt",
        "kms:Encrypt"
      ],
      "Resource": "*"
    }
  ]
}
```

**GCP IAM Roles:**
```
roles/compute.storageAdmin
roles/iam.serviceAccountUser
```

**Azure RBAC Roles:**
```
Contributor (for disk operations)
Storage Account Contributor (for snapshots)
```


## Risk Assessment and Mitigation

### High-Risk Areas

1. **Data Loss During Tier Migration**
   - Risk: Volume data corrupted during tier change
   - Mitigation: Always snapshot before migration, verify checksum after
   - Rollback: Restore from snapshot if verification fails

2. **Auto-Expansion Race Conditions**
   - Risk: Multiple controllers expand same volume simultaneously
   - Mitigation: Use leader election, add PVC annotations for locking
   - Testing: Chaos tests with multiple controller replicas

3. **CSI Driver Compatibility**
   - Risk: Driver versions incompatible with operator expectations
   - Mitigation: Version detection and capability probing at runtime
   - Fallback: Graceful degradation when features unavailable

4. **Cost Estimation Accuracy**
   - Risk: Inaccurate costs lead to budget overruns
   - Mitigation: Regular pricing updates, configurable pricing overrides
   - Monitoring: Alert when actual costs deviate >10% from estimates

5. **Snapshot Retention Policy Bugs**
   - Risk: Critical snapshots deleted prematurely
   - Mitigation: "Dry run" mode for retention enforcement, audit logs
   - Recovery: Require explicit confirmation before snapshot deletion

### Medium-Risk Areas

1. **Performance Degradation During Analysis**
   - Risk: Analytics engine consumes too many resources
   - Mitigation: Rate limiting, configurable sampling intervals
   - Monitoring: Controller CPU/memory usage alerts

2. **API Rate Limiting by Cloud Providers**
   - Risk: Too many API calls trigger rate limits
   - Mitigation: Exponential backoff, batch operations
   - Caching: Cache CSI driver capabilities and pricing data

3. **Metrics Cardinality Explosion**
   - Risk: High-cardinality labels overload Prometheus
   - Mitigation: Limit label values, aggregate by storage class
   - Documentation: Best practices for metrics retention

### Low-Risk Areas

1. **Dashboard Rendering Performance**
   - Risk: Grafana dashboards slow with many panels
   - Mitigation: Query optimization, configurable time ranges
   - Alternative: Provide simplified dashboard variant

2. **Documentation Staleness**
   - Risk: Docs don't reflect latest features
   - Mitigation: Doc generation from code comments
   - CI: Fail build if docs out of sync with CRD schema

