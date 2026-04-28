# StellarNode API Reference
> Auto-generated from the CRD OpenAPI schema. Do not edit manually.
> Re-generate with: `make generate-api-docs`

---

## Overview

| | |
|---|---|
| **CRD Name** | `stellarnodes.stellar.org` |
| **API Group** | `stellar.org` |
| **Kind** | `StellarNode` |
| **Plural** | `stellarnodes` |
| **Short Names** | `sn` |
| **Scope** | `Namespaced` |

## Version `v1alpha1`

| | |
|---|---|
| **Served** | `true` |
| **Storage** | `true` |
| **Subresources** | `status` |

### kubectl Printer Columns

| Name | Type | JSON Path |
|---|---|---|
| `Type` | `string` | `.spec.nodeType` |
| `Network` | `string` | `.spec.network` |
| `Ready` | `string` | `.status.conditions[?(@.type=='Ready')].status` |
| `Replicas` | `integer` | `.spec.replicas` |
| `Age` | `date` | `.metadata.creationTimestamp` |

## Spec Fields

Fields marked *(required)* must be present in every `StellarNode` manifest.


### `spec.alerting`

| | |
|---|---|
| **Path** | `spec.alerting` |
| **Type** | `boolean` |
| **Default** | `False` |

### `spec.autoscaling`

| | |
|---|---|
| **Path** | `spec.autoscaling` |
| **Type** | `object` |
| **Description** | Horizontal Pod Autoscaling configuration |
| **Nullable** | `true` |

#### `spec.autoscaling.behavior`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior` |
| **Type** | `object` |
| **Description** | Scaling behavior configuration for HPA |
| **Nullable** | `true` |

##### `spec.autoscaling.behavior.scaleDown`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior.scaleDown` |
| **Type** | `object` |
| **Description** | Scaling policy |
| **Nullable** | `true` |

###### `spec.autoscaling.behavior.scaleDown.policies`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior.scaleDown.policies` |
| **Type** | `array` of `object` |

###### `spec.autoscaling.behavior.scaleDown.stabilizationWindowSeconds`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior.scaleDown.stabilizationWindowSeconds` |
| **Type** | `integer` (int32) |
| **Nullable** | `true` |

##### `spec.autoscaling.behavior.scaleUp`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior.scaleUp` |
| **Type** | `object` |
| **Description** | Scaling policy |
| **Nullable** | `true` |

###### `spec.autoscaling.behavior.scaleUp.policies`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior.scaleUp.policies` |
| **Type** | `array` of `object` |

###### `spec.autoscaling.behavior.scaleUp.stabilizationWindowSeconds`

| | |
|---|---|
| **Path** | `spec.autoscaling.behavior.scaleUp.stabilizationWindowSeconds` |
| **Type** | `integer` (int32) |
| **Nullable** | `true` |

#### `spec.autoscaling.customMetrics`

| | |
|---|---|
| **Path** | `spec.autoscaling.customMetrics` |
| **Type** | `array` of `string` |

#### `spec.autoscaling.maxReplicas`

| | |
|---|---|
| **Path** | `spec.autoscaling.maxReplicas` |
| **Type** | `integer` (int32) |
| **Required** | *(required)* |

#### `spec.autoscaling.minReplicas`

| | |
|---|---|
| **Path** | `spec.autoscaling.minReplicas` |
| **Type** | `integer` (int32) |
| **Required** | *(required)* |

#### `spec.autoscaling.targetCpuUtilizationPercentage`

| | |
|---|---|
| **Path** | `spec.autoscaling.targetCpuUtilizationPercentage` |
| **Type** | `integer` (int32) |
| **Nullable** | `true` |

### `spec.crossCluster`

| | |
|---|---|
| **Path** | `spec.crossCluster` |
| **Type** | `object` |
| **Description** | Cross-cluster configuration for multi-cluster federation |
| **Nullable** | `true` |

#### `spec.crossCluster.autoDiscovery`

| | |
|---|---|
| **Path** | `spec.crossCluster.autoDiscovery` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.crossCluster.enabled`

| | |
|---|---|
| **Path** | `spec.crossCluster.enabled` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.crossCluster.externalName`

| | |
|---|---|
| **Path** | `spec.crossCluster.externalName` |
| **Type** | `object` |
| **Description** | ExternalName service configuration |
| **Nullable** | `true` |

##### `spec.crossCluster.externalName.createExternalNameServices`

| | |
|---|---|
| **Path** | `spec.crossCluster.externalName.createExternalNameServices` |
| **Type** | `boolean` |
| **Default** | `True` |

##### `spec.crossCluster.externalName.dnsProvider`

| | |
|---|---|
| **Path** | `spec.crossCluster.externalName.dnsProvider` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.crossCluster.externalName.externalDnsName`

| | |
|---|---|
| **Path** | `spec.crossCluster.externalName.externalDnsName` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.crossCluster.externalName.ttl`

| | |
|---|---|
| **Path** | `spec.crossCluster.externalName.ttl` |
| **Type** | `integer` (uint32) |
| **Default** | `300` |

#### `spec.crossCluster.healthCheck`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck` |
| **Type** | `object` |
| **Description** | Health check configuration for cross-cluster peers |
| **Nullable** | `true` |

##### `spec.crossCluster.healthCheck.enabled`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.enabled` |
| **Type** | `boolean` |
| **Default** | `True` |

##### `spec.crossCluster.healthCheck.failureThreshold`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.failureThreshold` |
| **Type** | `integer` (uint32) |
| **Default** | `3` |

##### `spec.crossCluster.healthCheck.intervalSeconds`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.intervalSeconds` |
| **Type** | `integer` (uint32) |
| **Default** | `30` |

##### `spec.crossCluster.healthCheck.latencyMeasurement`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.latencyMeasurement` |
| **Type** | `object` |
| **Description** | Latency measurement configuration |
| **Nullable** | `true` |

###### `spec.crossCluster.healthCheck.latencyMeasurement.enabled`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.latencyMeasurement.enabled` |
| **Type** | `boolean` |
| **Default** | `True` |

###### `spec.crossCluster.healthCheck.latencyMeasurement.method`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.latencyMeasurement.method` |
| **Type** | `string` |
| **Description** | Method for measuring cross-cluster latency |
| **Default** | `ping` |
| **Enum** | `ping`, `tcp`, `http`, `grpc` |

###### `spec.crossCluster.healthCheck.latencyMeasurement.percentile`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.latencyMeasurement.percentile` |
| **Type** | `integer` (uint8) |
| **Default** | `95` |

###### `spec.crossCluster.healthCheck.latencyMeasurement.sampleCount`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.latencyMeasurement.sampleCount` |
| **Type** | `integer` (uint32) |
| **Default** | `10` |

##### `spec.crossCluster.healthCheck.successThreshold`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.successThreshold` |
| **Type** | `integer` (uint32) |
| **Default** | `1` |

##### `spec.crossCluster.healthCheck.timeoutSeconds`

| | |
|---|---|
| **Path** | `spec.crossCluster.healthCheck.timeoutSeconds` |
| **Type** | `integer` (uint32) |
| **Default** | `5` |

#### `spec.crossCluster.latencyThresholdMs`

| | |
|---|---|
| **Path** | `spec.crossCluster.latencyThresholdMs` |
| **Type** | `integer` (uint32) |
| **Default** | `200` |

#### `spec.crossCluster.mode`

| | |
|---|---|
| **Path** | `spec.crossCluster.mode` |
| **Type** | `string` |
| **Description** | Cross-cluster networking mode |
| **Default** | `serviceMesh` |
| **Enum** | `serviceMesh`, `externalName`, `directIP` |

#### `spec.crossCluster.peerClusters`

| | |
|---|---|
| **Path** | `spec.crossCluster.peerClusters` |
| **Type** | `array` of `object` |

#### `spec.crossCluster.serviceMesh`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh` |
| **Type** | `object` |
| **Description** | Service mesh configuration for cross-cluster networking |
| **Nullable** | `true` |

##### `spec.crossCluster.serviceMesh.clusterSetId`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.clusterSetId` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.crossCluster.serviceMesh.meshType`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.meshType` |
| **Type** | `string` |
| **Description** | Supported service mesh types for cross-cluster networking |
| **Required** | *(required)* |
| **Enum** | `submariner`, `istio`, `linkerd`, `cilium` |

##### `spec.crossCluster.serviceMesh.mtlsEnabled`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.mtlsEnabled` |
| **Type** | `boolean` |
| **Default** | `True` |

##### `spec.crossCluster.serviceMesh.serviceExport`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.serviceExport` |
| **Type** | `object` |
| **Description** | Service export configuration |
| **Nullable** | `true` |

###### `spec.crossCluster.serviceMesh.serviceExport.enabled`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.serviceExport.enabled` |
| **Type** | `boolean` |
| **Default** | `True` |

###### `spec.crossCluster.serviceMesh.serviceExport.namespace`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.serviceExport.namespace` |
| **Type** | `string` |
| **Nullable** | `true` |

###### `spec.crossCluster.serviceMesh.serviceExport.serviceName`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.serviceExport.serviceName` |
| **Type** | `string` |
| **Nullable** | `true` |

###### `spec.crossCluster.serviceMesh.serviceExport.targetClusters`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.serviceExport.targetClusters` |
| **Type** | `array` of `string` |

##### `spec.crossCluster.serviceMesh.trafficPolicy`

| | |
|---|---|
| **Path** | `spec.crossCluster.serviceMesh.trafficPolicy` |
| **Type** | `string` |
| **Description** | Traffic policy for cross-cluster routing |
| **Default** | `localPreferred` |
| **Enum** | `localPreferred`, `global`, `localOnly`, `latencyBased` |

### `spec.customNetworkPassphrase`

| | |
|---|---|
| **Path** | `spec.customNetworkPassphrase` |
| **Type** | `string` |
| **Nullable** | `true` |

### `spec.cveHandling`

| | |
|---|---|
| **Path** | `spec.cveHandling` |
| **Type** | `object` |
| **Description** | CVE handling configuration for automated patching Enables scanning for vulnerabilities and automatic rollout of patched versions |
| **Nullable** | `true` |

#### `spec.cveHandling.canaryPassRateThreshold`

| | |
|---|---|
| **Path** | `spec.cveHandling.canaryPassRateThreshold` |
| **Type** | `number` (double) |
| **Default** | `100.0` |

#### `spec.cveHandling.canaryTestTimeoutSecs`

| | |
|---|---|
| **Path** | `spec.cveHandling.canaryTestTimeoutSecs` |
| **Type** | `integer` (uint64) |
| **Default** | `300` |

#### `spec.cveHandling.consensusHealthThreshold`

| | |
|---|---|
| **Path** | `spec.cveHandling.consensusHealthThreshold` |
| **Type** | `number` (double) |
| **Default** | `0.95` |

#### `spec.cveHandling.criticalOnly`

| | |
|---|---|
| **Path** | `spec.cveHandling.criticalOnly` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.cveHandling.enableAutoRollback`

| | |
|---|---|
| **Path** | `spec.cveHandling.enableAutoRollback` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.cveHandling.enabled`

| | |
|---|---|
| **Path** | `spec.cveHandling.enabled` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.cveHandling.scanIntervalSecs`

| | |
|---|---|
| **Path** | `spec.cveHandling.scanIntervalSecs` |
| **Type** | `integer` (uint64) |
| **Default** | `3600` |

### `spec.database`

| | |
|---|---|
| **Path** | `spec.database` |
| **Type** | `object` |
| **Description** | External database configuration for managed Postgres databases |
| **Nullable** | `true` |

#### `spec.database.secretKeyRef`

| | |
|---|---|
| **Path** | `spec.database.secretKeyRef` |
| **Type** | `object` |
| **Description** | Reference to a key within a Kubernetes Secret |
| **Required** | *(required)* |

##### `spec.database.secretKeyRef.key`

| | |
|---|---|
| **Path** | `spec.database.secretKeyRef.key` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.database.secretKeyRef.name`

| | |
|---|---|
| **Path** | `spec.database.secretKeyRef.name` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.dbMaintenanceConfig`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig` |
| **Type** | `object` |
| **Description** | Database maintenance configuration for automated vacuum and reindexing Enables periodic maintenance windows for performance optimization |
| **Nullable** | `true` |

#### `spec.dbMaintenanceConfig.autoReindex`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig.autoReindex` |
| **Type** | `boolean` |
| **Description** | Automatically reindex bloated tables |
| **Default** | `True` |

#### `spec.dbMaintenanceConfig.bloatThresholdPercent`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig.bloatThresholdPercent` |
| **Type** | `integer` (uint32) |
| **Description** | Bloat threshold percentage to trigger VACUUM FULL (default: 30) |
| **Default** | `30` |

#### `spec.dbMaintenanceConfig.enabled`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig.enabled` |
| **Type** | `boolean` |
| **Description** | Enable automated database maintenance |
| **Default** | `True` |

#### `spec.dbMaintenanceConfig.readPoolCoordination`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig.readPoolCoordination` |
| **Type** | `boolean` |
| **Description** | Coordination with read-pool for zero-downtime |
| **Default** | `True` |

#### `spec.dbMaintenanceConfig.windowDuration`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig.windowDuration` |
| **Type** | `string` |
| **Description** | Maintenance window duration (e.g., "2h") |
| **Required** | *(required)* |

#### `spec.dbMaintenanceConfig.windowStart`

| | |
|---|---|
| **Path** | `spec.dbMaintenanceConfig.windowStart` |
| **Type** | `string` |
| **Description** | Maintenance window start time (24h format, e.g., "02:00") Maintenance will only trigger during this window |
| **Required** | *(required)* |

### `spec.drConfig`

| | |
|---|---|
| **Path** | `spec.drConfig` |
| **Type** | `object` |
| **Description** | Configuration for multi-cluster disaster recovery |
| **Nullable** | `true` |

#### `spec.drConfig.drillSchedule`

| | |
|---|---|
| **Path** | `spec.drConfig.drillSchedule` |
| **Type** | `object` |
| **Description** | Configuration for automated DR drill scheduling |
| **Nullable** | `true` |

##### `spec.drConfig.drillSchedule.autoRollback`

| | |
|---|---|
| **Path** | `spec.drConfig.drillSchedule.autoRollback` |
| **Type** | `boolean` |
| **Description** | Whether to automatically rollback after drill completion |
| **Default** | `True` |

##### `spec.drConfig.drillSchedule.dryRun`

| | |
|---|---|
| **Path** | `spec.drConfig.drillSchedule.dryRun` |
| **Type** | `boolean` |
| **Description** | Whether to actually perform failover or just simulate it (dry-run) |
| **Default** | `False` |

##### `spec.drConfig.drillSchedule.rollbackDelaySeconds`

| | |
|---|---|
| **Path** | `spec.drConfig.drillSchedule.rollbackDelaySeconds` |
| **Type** | `integer` (uint32) |
| **Description** | Rollback delay after drill completion (seconds) |
| **Default** | `60` |

##### `spec.drConfig.drillSchedule.schedule`

| | |
|---|---|
| **Path** | `spec.drConfig.drillSchedule.schedule` |
| **Type** | `string` |
| **Description** | Cron expression for drill scheduling (e.g., "0 2 * * 0" for weekly Sunday 2 AM) |
| **Required** | *(required)* |

##### `spec.drConfig.drillSchedule.timeoutSeconds`

| | |
|---|---|
| **Path** | `spec.drConfig.drillSchedule.timeoutSeconds` |
| **Type** | `integer` (uint32) |
| **Description** | Maximum time to wait for failover to complete (seconds) |
| **Default** | `300` |

#### `spec.drConfig.enabled`

| | |
|---|---|
| **Path** | `spec.drConfig.enabled` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.drConfig.failoverDns`

| | |
|---|---|
| **Path** | `spec.drConfig.failoverDns` |
| **Type** | `object` |
| **Description** | ExternalDNS configuration |
| **Nullable** | `true` |

##### `spec.drConfig.failoverDns.annotations`

| | |
|---|---|
| **Path** | `spec.drConfig.failoverDns.annotations` |
| **Type** | `object` |
| **Nullable** | `true` |

##### `spec.drConfig.failoverDns.hostname`

| | |
|---|---|
| **Path** | `spec.drConfig.failoverDns.hostname` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.drConfig.failoverDns.provider`

| | |
|---|---|
| **Path** | `spec.drConfig.failoverDns.provider` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.drConfig.failoverDns.ttl`

| | |
|---|---|
| **Path** | `spec.drConfig.failoverDns.ttl` |
| **Type** | `integer` (uint32) |
| **Default** | `300` |

#### `spec.drConfig.healthCheckInterval`

| | |
|---|---|
| **Path** | `spec.drConfig.healthCheckInterval` |
| **Type** | `integer` (uint32) |
| **Default** | `30` |

#### `spec.drConfig.peerClusterId`

| | |
|---|---|
| **Path** | `spec.drConfig.peerClusterId` |
| **Type** | `string` |
| **Required** | *(required)* |

#### `spec.drConfig.role`

| | |
|---|---|
| **Path** | `spec.drConfig.role` |
| **Type** | `string` |
| **Description** | Role of a node in a DR configuration |
| **Required** | *(required)* |
| **Enum** | `primary`, `standby` |

#### `spec.drConfig.syncStrategy`

| | |
|---|---|
| **Path** | `spec.drConfig.syncStrategy` |
| **Type** | `string` |
| **Description** | Synchronization strategy for hot standby nodes |
| **Default** | `consensus` |
| **Enum** | `consensus`, `peertracking`, `archivesync` |

### `spec.forensicSnapshot`

| | |
|---|---|
| **Path** | `spec.forensicSnapshot` |
| **Type** | `object` |
| **Description** | Forensic snapshot: set `metadata.annotations["stellar.org/request-forensic-snapshot"]="true"` to trigger a one-shot capture (PCAP, optional core dump) uploaded to S3. |
| **Nullable** | `true` |

#### `spec.forensicSnapshot.credentialsSecretRef`

| | |
|---|---|
| **Path** | `spec.forensicSnapshot.credentialsSecretRef` |
| **Type** | `string` |
| **Description** | Secret in the same namespace with `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` when not using IRSA/instance roles. |
| **Nullable** | `true` |

#### `spec.forensicSnapshot.enableShareProcessNamespace`

| | |
|---|---|
| **Path** | `spec.forensicSnapshot.enableShareProcessNamespace` |
| **Type** | `boolean` |
| **Description** | Set `shareProcessNamespace: true` on validator pods so the capture container can see `stellar-core` for core dumps (recommended for forensic workflows). |
| **Default** | `False` |

#### `spec.forensicSnapshot.kmsKeyId`

| | |
|---|---|
| **Path** | `spec.forensicSnapshot.kmsKeyId` |
| **Type** | `string` |
| **Description** | Optional KMS key id for SSE-KMS (`aws s3 cp --sse aws:kms`). |
| **Nullable** | `true` |

#### `spec.forensicSnapshot.s3Bucket`

| | |
|---|---|
| **Path** | `spec.forensicSnapshot.s3Bucket` |
| **Type** | `string` |
| **Description** | Target S3 bucket for the encrypted forensic tarball. |
| **Required** | *(required)* |

#### `spec.forensicSnapshot.s3Prefix`

| | |
|---|---|
| **Path** | `spec.forensicSnapshot.s3Prefix` |
| **Type** | `string` |
| **Nullable** | `true` |

### `spec.globalDiscovery`

| | |
|---|---|
| **Path** | `spec.globalDiscovery` |
| **Type** | `object` |
| **Description** | Global discovery configuration for cross-cluster discovery |
| **Nullable** | `true` |

#### `spec.globalDiscovery.enabled`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.enabled` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.globalDiscovery.externalDns`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.externalDns` |
| **Type** | `object` |
| **Description** | ExternalDNS configuration |
| **Nullable** | `true` |

##### `spec.globalDiscovery.externalDns.annotations`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.externalDns.annotations` |
| **Type** | `object` |
| **Nullable** | `true` |

##### `spec.globalDiscovery.externalDns.hostname`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.externalDns.hostname` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.globalDiscovery.externalDns.provider`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.externalDns.provider` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.globalDiscovery.externalDns.ttl`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.externalDns.ttl` |
| **Type** | `integer` (uint32) |
| **Default** | `300` |

#### `spec.globalDiscovery.priority`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.priority` |
| **Type** | `integer` (uint32) |
| **Default** | `100` |

#### `spec.globalDiscovery.region`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.region` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.globalDiscovery.serviceMesh`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.serviceMesh` |
| **Type** | `object` |
| **Description** | Service mesh integration configuration |
| **Nullable** | `true` |

##### `spec.globalDiscovery.serviceMesh.meshType`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.serviceMesh.meshType` |
| **Type** | `string` |
| **Description** | Supported service mesh implementations |
| **Required** | *(required)* |
| **Enum** | `istio`, `linkerd`, `consul` |

##### `spec.globalDiscovery.serviceMesh.mtlsMode`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.serviceMesh.mtlsMode` |
| **Type** | `string` |
| **Description** | mTLS enforcement mode |
| **Default** | `PERMISSIVE` |
| **Enum** | `DISABLE`, `PERMISSIVE`, `STRICT` |

##### `spec.globalDiscovery.serviceMesh.sidecarInjection`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.serviceMesh.sidecarInjection` |
| **Type** | `boolean` |
| **Default** | `True` |

##### `spec.globalDiscovery.serviceMesh.virtualServiceHost`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.serviceMesh.virtualServiceHost` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.globalDiscovery.topologyAwareHints`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.topologyAwareHints` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.globalDiscovery.zone`

| | |
|---|---|
| **Path** | `spec.globalDiscovery.zone` |
| **Type** | `string` |
| **Nullable** | `true` |

### `spec.historyMode`

| | |
|---|---|
| **Path** | `spec.historyMode` |
| **Type** | `string` |
| **Description** | History mode for the node |
| **Default** | `Recent` |
| **Enum** | `Full`, `Recent` |

### `spec.horizonConfig`

| | |
|---|---|
| **Path** | `spec.horizonConfig` |
| **Type** | `object` |
| **Description** | Horizon API server configuration |
| **Nullable** | `true` |

#### `spec.horizonConfig.autoMigration`

| | |
|---|---|
| **Path** | `spec.horizonConfig.autoMigration` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.horizonConfig.databaseSecretRef`

| | |
|---|---|
| **Path** | `spec.horizonConfig.databaseSecretRef` |
| **Type** | `string` |
| **Required** | *(required)* |

#### `spec.horizonConfig.enableExperimentalIngestion`

| | |
|---|---|
| **Path** | `spec.horizonConfig.enableExperimentalIngestion` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.horizonConfig.enableIngest`

| | |
|---|---|
| **Path** | `spec.horizonConfig.enableIngest` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.horizonConfig.ingestWorkers`

| | |
|---|---|
| **Path** | `spec.horizonConfig.ingestWorkers` |
| **Type** | `integer` (uint32) |
| **Default** | `1` |

#### `spec.horizonConfig.stellarCoreUrl`

| | |
|---|---|
| **Path** | `spec.horizonConfig.stellarCoreUrl` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.ingress`

| | |
|---|---|
| **Path** | `spec.ingress` |
| **Type** | `object` |
| **Description** | Ingress configuration |
| **Nullable** | `true` |

#### `spec.ingress.annotations`

| | |
|---|---|
| **Path** | `spec.ingress.annotations` |
| **Type** | `object` |
| **Nullable** | `true` |

#### `spec.ingress.certManagerClusterIssuer`

| | |
|---|---|
| **Path** | `spec.ingress.certManagerClusterIssuer` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.ingress.certManagerIssuer`

| | |
|---|---|
| **Path** | `spec.ingress.certManagerIssuer` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.ingress.className`

| | |
|---|---|
| **Path** | `spec.ingress.className` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.ingress.hosts`

| | |
|---|---|
| **Path** | `spec.ingress.hosts` |
| **Type** | `array` of `object` |
| **Required** | *(required)* |

#### `spec.ingress.tlsSecretName`

| | |
|---|---|
| **Path** | `spec.ingress.tlsSecretName` |
| **Type** | `string` |
| **Nullable** | `true` |

### `spec.loadBalancer`

| | |
|---|---|
| **Path** | `spec.loadBalancer` |
| **Type** | `object` |
| **Description** | Load balancer configuration for external access (e.g. MetalLB) |
| **Nullable** | `true` |

#### `spec.loadBalancer.addressPool`

| | |
|---|---|
| **Path** | `spec.loadBalancer.addressPool` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.loadBalancer.annotations`

| | |
|---|---|
| **Path** | `spec.loadBalancer.annotations` |
| **Type** | `object` |
| **Nullable** | `true` |

#### `spec.loadBalancer.bgp`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp` |
| **Type** | `object` |
| **Description** | BGP configuration for MetalLB anycast routing |
| **Nullable** | `true` |

##### `spec.loadBalancer.bgp.advertisement`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.advertisement` |
| **Type** | `object` |
| **Description** | BGP advertisement configuration |
| **Nullable** | `true` |

###### `spec.loadBalancer.bgp.advertisement.aggregationLength`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.advertisement.aggregationLength` |
| **Type** | `integer` (uint8) |
| **Default** | `32` |

###### `spec.loadBalancer.bgp.advertisement.aggregationLengthV6`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.advertisement.aggregationLengthV6` |
| **Type** | `integer` (uint8) |
| **Default** | `128` |

###### `spec.loadBalancer.bgp.advertisement.localPref`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.advertisement.localPref` |
| **Type** | `integer` (uint32) |
| **Nullable** | `true` |

###### `spec.loadBalancer.bgp.advertisement.nodeSelectors`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.advertisement.nodeSelectors` |
| **Type** | `object` |
| **Nullable** | `true` |

##### `spec.loadBalancer.bgp.bfdEnabled`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.bfdEnabled` |
| **Type** | `boolean` |
| **Default** | `False` |

##### `spec.loadBalancer.bgp.bfdProfile`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.bfdProfile` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.loadBalancer.bgp.communities`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.communities` |
| **Type** | `array` of `string` |

##### `spec.loadBalancer.bgp.largeCommunities`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.largeCommunities` |
| **Type** | `array` of `string` |

##### `spec.loadBalancer.bgp.localAsn`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.localAsn` |
| **Type** | `integer` (uint32) |
| **Required** | *(required)* |

##### `spec.loadBalancer.bgp.nodeSelectors`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.nodeSelectors` |
| **Type** | `object` |
| **Nullable** | `true` |

##### `spec.loadBalancer.bgp.peers`

| | |
|---|---|
| **Path** | `spec.loadBalancer.bgp.peers` |
| **Type** | `array` of `object` |

#### `spec.loadBalancer.enabled`

| | |
|---|---|
| **Path** | `spec.loadBalancer.enabled` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.loadBalancer.externalTrafficPolicy`

| | |
|---|---|
| **Path** | `spec.loadBalancer.externalTrafficPolicy` |
| **Type** | `string` |
| **Description** | External traffic policy for LoadBalancer services |
| **Default** | `Cluster` |
| **Enum** | `Cluster`, `Local` |

#### `spec.loadBalancer.healthCheckEnabled`

| | |
|---|---|
| **Path** | `spec.loadBalancer.healthCheckEnabled` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.loadBalancer.healthCheckPort`

| | |
|---|---|
| **Path** | `spec.loadBalancer.healthCheckPort` |
| **Type** | `integer` (int32) |
| **Default** | `9100` |

#### `spec.loadBalancer.loadBalancerIp`

| | |
|---|---|
| **Path** | `spec.loadBalancer.loadBalancerIp` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.loadBalancer.mode`

| | |
|---|---|
| **Path** | `spec.loadBalancer.mode` |
| **Type** | `string` |
| **Description** | Load balancer mode selection |
| **Default** | `L2` |
| **Enum** | `L2`, `BGP` |

### `spec.maintenanceMode`

| | |
|---|---|
| **Path** | `spec.maintenanceMode` |
| **Type** | `boolean` |
| **Default** | `False` |

### `spec.managedDatabase`

| | |
|---|---|
| **Path** | `spec.managedDatabase` |
| **Type** | `object` |
| **Description** | Configuration for managed High-Availability Postgres clusters via CloudNativePG |
| **Nullable** | `true` |

#### `spec.managedDatabase.backup`

| | |
|---|---|
| **Path** | `spec.managedDatabase.backup` |
| **Type** | `object` |
| **Description** | Backup configuration for managed databases using Barman |
| **Nullable** | `true` |

##### `spec.managedDatabase.backup.credentialsSecretRef`

| | |
|---|---|
| **Path** | `spec.managedDatabase.backup.credentialsSecretRef` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.managedDatabase.backup.destinationPath`

| | |
|---|---|
| **Path** | `spec.managedDatabase.backup.destinationPath` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.managedDatabase.backup.enabled`

| | |
|---|---|
| **Path** | `spec.managedDatabase.backup.enabled` |
| **Type** | `boolean` |
| **Default** | `True` |

##### `spec.managedDatabase.backup.retentionPolicy`

| | |
|---|---|
| **Path** | `spec.managedDatabase.backup.retentionPolicy` |
| **Type** | `string` |
| **Default** | `30d` |

#### `spec.managedDatabase.instances`

| | |
|---|---|
| **Path** | `spec.managedDatabase.instances` |
| **Type** | `integer` (int32) |
| **Default** | `3` |

#### `spec.managedDatabase.pooling`

| | |
|---|---|
| **Path** | `spec.managedDatabase.pooling` |
| **Type** | `object` |
| **Description** | pgBouncer connection pooling configuration |
| **Nullable** | `true` |

##### `spec.managedDatabase.pooling.defaultPoolSize`

| | |
|---|---|
| **Path** | `spec.managedDatabase.pooling.defaultPoolSize` |
| **Type** | `integer` (int32) |
| **Default** | `20` |

##### `spec.managedDatabase.pooling.enabled`

| | |
|---|---|
| **Path** | `spec.managedDatabase.pooling.enabled` |
| **Type** | `boolean` |
| **Default** | `True` |

##### `spec.managedDatabase.pooling.maxClientConn`

| | |
|---|---|
| **Path** | `spec.managedDatabase.pooling.maxClientConn` |
| **Type** | `integer` (int32) |
| **Default** | `1000` |

##### `spec.managedDatabase.pooling.poolMode`

| | |
|---|---|
| **Path** | `spec.managedDatabase.pooling.poolMode` |
| **Type** | `string` |
| **Description** | pgBouncer pooling modes |
| **Default** | `transaction` |
| **Enum** | `session`, `transaction`, `statement` |

##### `spec.managedDatabase.pooling.replicas`

| | |
|---|---|
| **Path** | `spec.managedDatabase.pooling.replicas` |
| **Type** | `integer` (int32) |
| **Default** | `2` |

#### `spec.managedDatabase.postgresVersion`

| | |
|---|---|
| **Path** | `spec.managedDatabase.postgresVersion` |
| **Type** | `string` |
| **Default** | `16` |

#### `spec.managedDatabase.storage`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage` |
| **Type** | `object` |
| **Description** | Storage configuration for persistent data |
| **Required** | *(required)* |

##### `spec.managedDatabase.storage.annotations`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage.annotations` |
| **Type** | `object` |
| **Nullable** | `true` |

##### `spec.managedDatabase.storage.mode`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage.mode` |
| **Type** | `string` |
| **Description** | Storage mode for persistent data |
| **Default** | `PersistentVolume` |
| **Enum** | `PersistentVolume`, `Local` |

##### `spec.managedDatabase.storage.nodeAffinity`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage.nodeAffinity` |
| **Type** | `object` |
| **Description** | Node affinity for local storage mode (optional) |

##### `spec.managedDatabase.storage.retentionPolicy`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage.retentionPolicy` |
| **Type** | `string` |
| **Description** | PVC retention policy on node deletion |
| **Default** | `Delete` |
| **Enum** | `Delete`, `Retain` |

##### `spec.managedDatabase.storage.size`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage.size` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.managedDatabase.storage.storageClass`

| | |
|---|---|
| **Path** | `spec.managedDatabase.storage.storageClass` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.maxUnavailable`

| | |
|---|---|
| **Path** | `spec.maxUnavailable` |
| **Type** | `object` |
| **Description** | IntOrString |
| **Required** | *(required)* |

### `spec.minAvailable`

| | |
|---|---|
| **Path** | `spec.minAvailable` |
| **Type** | `object` |
| **Description** | IntOrString |
| **Required** | *(required)* |

### `spec.network`

| | |
|---|---|
| **Path** | `spec.network` |
| **Type** | `string` |
| **Description** | Target Stellar network |
| **Required** | *(required)* |
| **Enum** | `mainnet`, `testnet`, `futurenet`, `custom` |

### `spec.networkPolicy`

| | |
|---|---|
| **Path** | `spec.networkPolicy` |
| **Type** | `object` |
| **Description** | Network Policy configuration |
| **Nullable** | `true` |

#### `spec.networkPolicy.allowCidrs`

| | |
|---|---|
| **Path** | `spec.networkPolicy.allowCidrs` |
| **Type** | `array` of `string` |

#### `spec.networkPolicy.allowMetricsScrape`

| | |
|---|---|
| **Path** | `spec.networkPolicy.allowMetricsScrape` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.networkPolicy.allowNamespaces`

| | |
|---|---|
| **Path** | `spec.networkPolicy.allowNamespaces` |
| **Type** | `array` of `string` |

#### `spec.networkPolicy.allowPodSelector`

| | |
|---|---|
| **Path** | `spec.networkPolicy.allowPodSelector` |
| **Type** | `object` |
| **Nullable** | `true` |

#### `spec.networkPolicy.enabled`

| | |
|---|---|
| **Path** | `spec.networkPolicy.enabled` |
| **Type** | `boolean` |
| **Default** | `False` |

#### `spec.networkPolicy.metricsNamespace`

| | |
|---|---|
| **Path** | `spec.networkPolicy.metricsNamespace` |
| **Type** | `string` |
| **Default** | `monitoring` |

### `spec.nodeType`

| | |
|---|---|
| **Path** | `spec.nodeType` |
| **Type** | `string` |
| **Description** | Supported Stellar node types |
| **Required** | *(required)* |
| **Enum** | `Validator`, `Horizon`, `SorobanRpc` |

### `spec.ociSnapshot`

| | |
|---|---|
| **Path** | `spec.ociSnapshot` |
| **Type** | `object` |
| **Description** | OCI-based ledger snapshot sync for multi-region bootstrapping |
| **Nullable** | `true` |

#### `spec.ociSnapshot.credentialSecretName`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.credentialSecretName` |
| **Type** | `string` |
| **Description** | Name of a K8s Secret in the same namespace containing Docker registry credentials as `config.json` (standard `~/.docker/config.json` format). |
| **Required** | *(required)* |

#### `spec.ociSnapshot.enabled`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.enabled` |
| **Type** | `boolean` |
| **Description** | Whether the OCI snapshot feature is enabled (default: false) |
| **Default** | `False` |

#### `spec.ociSnapshot.fixedTag`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.fixedTag` |
| **Type** | `string` |
| **Description** | Fixed tag to use when `tag_strategy` is `Fixed` (e.g. `latest`) |
| **Nullable** | `true` |

#### `spec.ociSnapshot.image`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.image` |
| **Type** | `string` |
| **Description** | Image name within the registry, e.g. `myorg/stellar-snapshot` |
| **Required** | *(required)* |

#### `spec.ociSnapshot.pull`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.pull` |
| **Type** | `boolean` |
| **Description** | Enable pulling a snapshot to bootstrap a new node's PVC (default: false) |
| **Default** | `False` |

#### `spec.ociSnapshot.pullImageRef`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.pullImageRef` |
| **Type** | `string` |
| **Description** | Image reference to pull from (full `registry/image:tag` string). Required when `pull = true`; if omitted the operator constructs the reference from `registry`, `image`, and `tag_strategy`. |
| **Nullable** | `true` |

#### `spec.ociSnapshot.push`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.push` |
| **Type** | `boolean` |
| **Description** | Enable pushing snapshots to the registry (default: false) |
| **Default** | `False` |

#### `spec.ociSnapshot.registry`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.registry` |
| **Type** | `string` |
| **Description** | OCI registry host, e.g. `ghcr.io` or `registry-1.docker.io` |
| **Required** | *(required)* |

#### `spec.ociSnapshot.tagStrategy`

| | |
|---|---|
| **Path** | `spec.ociSnapshot.tagStrategy` |
| **Type** | `string` |
| **Description** | Tag used when pushing/pulling the snapshot image. With `LatestLedger` the tag is `snapshot-<ledger_seq>`; with `Fixed` the literal `fixed_tag` value is used. |
| **Default** | `latestLedger` |
| **Enum** | `latestLedger`, `fixed` |

### `spec.podAntiAffinity`

| | |
|---|---|
| **Path** | `spec.podAntiAffinity` |
| **Type** | `string` |
| **Description** | When not `Disabled`, the operator adds default pod anti-affinity so pods with the same `stellar-network` label (and same component) are not co-located on one node. |
| **Default** | `Hard` |
| **Enum** | `Hard`, `Soft`, `Disabled` |

### `spec.readPoolEndpoint`

| | |
|---|---|
| **Path** | `spec.readPoolEndpoint` |
| **Type** | `string` |
| **Description** | DNS endpoint for the read-replica pool Service. |
| **Nullable** | `true` |

### `spec.readReplicaConfig`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig` |
| **Type** | `object` |
| **Description** | Read replica pool configuration for horizontal scaling Enables creating read-only replicas with traffic routing strategies |
| **Nullable** | `true` |

#### `spec.readReplicaConfig.archiveSharding`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.archiveSharding` |
| **Type** | `boolean` |
| **Description** | Enable history archive sharding When true, replicas serve different archives to balance bandwidth |
| **Default** | `False` |

#### `spec.readReplicaConfig.replicas`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.replicas` |
| **Type** | `integer` (int32) |
| **Description** | Number of read-only replicas |
| **Default** | `1` |

#### `spec.readReplicaConfig.resources`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources` |
| **Type** | `object` |
| **Description** | Compute resource requirements for read replicas |
| **Default** | `{'limits': {'cpu': '2', 'memory': '4Gi'}, 'requests': {'cpu': '500m', 'memory': '1Gi'}}` |

##### `spec.readReplicaConfig.resources.limits`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources.limits` |
| **Type** | `object` |
| **Description** | Resource specification for CPU and memory |
| **Required** | *(required)* |

###### `spec.readReplicaConfig.resources.limits.cpu`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources.limits.cpu` |
| **Type** | `string` |
| **Required** | *(required)* |

###### `spec.readReplicaConfig.resources.limits.memory`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources.limits.memory` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.readReplicaConfig.resources.requests`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources.requests` |
| **Type** | `object` |
| **Description** | Resource specification for CPU and memory |
| **Required** | *(required)* |

###### `spec.readReplicaConfig.resources.requests.cpu`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources.requests.cpu` |
| **Type** | `string` |
| **Required** | *(required)* |

###### `spec.readReplicaConfig.resources.requests.memory`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.resources.requests.memory` |
| **Type** | `string` |
| **Required** | *(required)* |

#### `spec.readReplicaConfig.strategy`

| | |
|---|---|
| **Path** | `spec.readReplicaConfig.strategy` |
| **Type** | `string` |
| **Description** | Load balancing strategy |
| **Default** | `RoundRobin` |
| **Enum** | `RoundRobin`, `FreshnessPreferred` |

### `spec.replicas`

| | |
|---|---|
| **Path** | `spec.replicas` |
| **Type** | `integer` (int32) |
| **Default** | `1` |

### `spec.resources`

| | |
|---|---|
| **Path** | `spec.resources` |
| **Type** | `object` |
| **Description** | Kubernetes-style resource requirements |
| **Default** | `{'limits': {'cpu': '2', 'memory': '4Gi'}, 'requests': {'cpu': '500m', 'memory': '1Gi'}}` |

#### `spec.resources.limits`

| | |
|---|---|
| **Path** | `spec.resources.limits` |
| **Type** | `object` |
| **Description** | Resource specification for CPU and memory |
| **Required** | *(required)* |

##### `spec.resources.limits.cpu`

| | |
|---|---|
| **Path** | `spec.resources.limits.cpu` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.resources.limits.memory`

| | |
|---|---|
| **Path** | `spec.resources.limits.memory` |
| **Type** | `string` |
| **Required** | *(required)* |

#### `spec.resources.requests`

| | |
|---|---|
| **Path** | `spec.resources.requests` |
| **Type** | `object` |
| **Description** | Resource specification for CPU and memory |
| **Required** | *(required)* |

##### `spec.resources.requests.cpu`

| | |
|---|---|
| **Path** | `spec.resources.requests.cpu` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.resources.requests.memory`

| | |
|---|---|
| **Path** | `spec.resources.requests.memory` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.restoreFromSnapshot`

| | |
|---|---|
| **Path** | `spec.restoreFromSnapshot` |
| **Type** | `object` |
| **Description** | Bootstrap this node from an existing VolumeSnapshot instead of an empty volume (Validator only). The PVC will be created from the specified snapshot for near-instant startup. |
| **Nullable** | `true` |

#### `spec.restoreFromSnapshot.namespace`

| | |
|---|---|
| **Path** | `spec.restoreFromSnapshot.namespace` |
| **Type** | `string` |
| **Description** | Optional: namespace of the VolumeSnapshot if different from the StellarNode. Requires CrossNamespaceVolumeDataSource where supported. |
| **Nullable** | `true` |

#### `spec.restoreFromSnapshot.volumeSnapshotName`

| | |
|---|---|
| **Path** | `spec.restoreFromSnapshot.volumeSnapshotName` |
| **Type** | `string` |
| **Description** | Name of the VolumeSnapshot to restore from (must exist in the same namespace as the StellarNode). |
| **Required** | *(required)* |

### `spec.serviceMesh`

| | |
|---|---|
| **Path** | `spec.serviceMesh` |
| **Type** | `object` |
| **Description** | Service mesh configuration (Istio/Linkerd) for mTLS and advanced traffic control |
| **Nullable** | `true` |

#### `spec.serviceMesh.istio`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio` |
| **Type** | `object` |
| **Description** | Istio-specific configuration |
| **Nullable** | `true` |

##### `spec.serviceMesh.istio.circuitBreaker`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.circuitBreaker` |
| **Type** | `object` |
| **Description** | Circuit breaker configuration for outlier detection |
| **Nullable** | `true` |

###### `spec.serviceMesh.istio.circuitBreaker.consecutiveErrors`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.circuitBreaker.consecutiveErrors` |
| **Type** | `integer` (uint32) |
| **Description** | Number of consecutive errors before opening circuit |
| **Default** | `5` |

###### `spec.serviceMesh.istio.circuitBreaker.minRequestVolume`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.circuitBreaker.minRequestVolume` |
| **Type** | `integer` (uint32) |
| **Description** | Minimum request volume before applying circuit breaking |
| **Default** | `10` |

###### `spec.serviceMesh.istio.circuitBreaker.timeWindowSecs`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.circuitBreaker.timeWindowSecs` |
| **Type** | `integer` (uint32) |
| **Description** | Time window in seconds for counting errors |
| **Default** | `30` |

##### `spec.serviceMesh.istio.mtlsMode`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.mtlsMode` |
| **Type** | `string` |
| **Description** | mTLS mode (STRICT or PERMISSIVE) |
| **Default** | `STRICT` |
| **Enum** | `STRICT`, `PERMISSIVE` |

##### `spec.serviceMesh.istio.retries`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.retries` |
| **Type** | `object` |
| **Description** | Retry policy for failed requests |
| **Nullable** | `true` |

###### `spec.serviceMesh.istio.retries.backoffMs`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.retries.backoffMs` |
| **Type** | `integer` (uint32) |
| **Description** | Backoff duration in milliseconds |
| **Default** | `25` |

###### `spec.serviceMesh.istio.retries.maxRetries`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.retries.maxRetries` |
| **Type** | `integer` (uint32) |
| **Description** | Maximum number of retries |
| **Default** | `3` |

###### `spec.serviceMesh.istio.retries.retryableStatusCodes`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.retries.retryableStatusCodes` |
| **Type** | `array` of `integer` |
| **Description** | Retryable status codes (e.g., 503, 504) |
| **Default** | `[]` |

##### `spec.serviceMesh.istio.timeoutSecs`

| | |
|---|---|
| **Path** | `spec.serviceMesh.istio.timeoutSecs` |
| **Type** | `integer` (uint32) |
| **Description** | VirtualService timeout in seconds |
| **Default** | `30` |

#### `spec.serviceMesh.linkerd`

| | |
|---|---|
| **Path** | `spec.serviceMesh.linkerd` |
| **Type** | `object` |
| **Description** | Linkerd-specific configuration |
| **Nullable** | `true` |

##### `spec.serviceMesh.linkerd.autoMtls`

| | |
|---|---|
| **Path** | `spec.serviceMesh.linkerd.autoMtls` |
| **Type** | `boolean` |
| **Description** | Enable automatic mTLS |
| **Default** | `True` |

##### `spec.serviceMesh.linkerd.policyMode`

| | |
|---|---|
| **Path** | `spec.serviceMesh.linkerd.policyMode` |
| **Type** | `string` |
| **Description** | Policy mode (deny, audit, allow) |
| **Default** | `allow` |

#### `spec.serviceMesh.sidecarInjection`

| | |
|---|---|
| **Path** | `spec.serviceMesh.sidecarInjection` |
| **Type** | `boolean` |
| **Description** | Enable sidecar injection for this node |
| **Default** | `True` |

### `spec.snapshotSchedule`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule` |
| **Type** | `object` |
| **Description** | Schedule and options for taking CSI VolumeSnapshots of the node's data PVC (Validator only). Enables zero-downtime backups and creating new nodes from snapshots. |
| **Nullable** | `true` |

#### `spec.snapshotSchedule.flushBeforeSnapshot`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.flushBeforeSnapshot` |
| **Type** | `boolean` |
| **Description** | If true, the operator will attempt to flush/lock the Stellar database briefly before creating the snapshot (e.g. via stellar-core HTTP or exec). Requires the node to be healthy. |
| **Default** | `False` |

#### `spec.snapshotSchedule.retentionCount`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.retentionCount` |
| **Type** | `integer` (uint32) |
| **Description** | Maximum number of snapshots to retain per node. Oldest snapshots are deleted when exceeded. 0 means no limit. |
| **Default** | `0` |

#### `spec.snapshotSchedule.schedule`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.schedule` |
| **Type** | `string` |
| **Description** | Cron expression for scheduled snapshots (e.g. "0 2 * * *" for daily at 2 AM). If unset, snapshots are only taken when triggered via annotation `stellar.org/request-snapshot: "true"`. |
| **Nullable** | `true` |

#### `spec.snapshotSchedule.volumeSnapshotClassName`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.volumeSnapshotClassName` |
| **Type** | `string` |
| **Description** | VolumeSnapshotClass name. If unset, the default class for the PVC's driver is used. |
| **Nullable** | `true` |

### `spec.sorobanConfig`

| | |
|---|---|
| **Path** | `spec.sorobanConfig` |
| **Type** | `object` |
| **Description** | Soroban RPC server configuration |
| **Nullable** | `true` |

#### `spec.sorobanConfig.captiveCoreConfig`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreConfig` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.sorobanConfig.captiveCoreStructuredConfig`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig` |
| **Type** | `object` |
| **Description** | Captive Core configuration for Soroban RPC |
| **Nullable** | `true` |

##### `spec.sorobanConfig.captiveCoreStructuredConfig.additionalConfig`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig.additionalConfig` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.sorobanConfig.captiveCoreStructuredConfig.historyArchiveUrls`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig.historyArchiveUrls` |
| **Type** | `array` of `string` |
| **Default** | `[]` |

##### `spec.sorobanConfig.captiveCoreStructuredConfig.httpPort`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig.httpPort` |
| **Type** | `integer` (uint16) |
| **Nullable** | `true` |

##### `spec.sorobanConfig.captiveCoreStructuredConfig.logLevel`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig.logLevel` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.sorobanConfig.captiveCoreStructuredConfig.networkPassphrase`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig.networkPassphrase` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.sorobanConfig.captiveCoreStructuredConfig.peerPort`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreStructuredConfig.peerPort` |
| **Type** | `integer` (uint16) |
| **Nullable** | `true` |

#### `spec.sorobanConfig.enablePreflight`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.enablePreflight` |
| **Type** | `boolean` |
| **Default** | `True` |

#### `spec.sorobanConfig.maxEventsPerRequest`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.maxEventsPerRequest` |
| **Type** | `integer` (uint32) |
| **Default** | `10000` |

#### `spec.sorobanConfig.stellarCoreUrl`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.stellarCoreUrl` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.storage`

| | |
|---|---|
| **Path** | `spec.storage` |
| **Type** | `object` |
| **Description** | Storage configuration for persistent data |
| **Default** | `{'mode': 'PersistentVolume', 'retentionPolicy': 'Delete', 'size': '100Gi', 'storageClass': 'standard'}` |

#### `spec.storage.annotations`

| | |
|---|---|
| **Path** | `spec.storage.annotations` |
| **Type** | `object` |
| **Nullable** | `true` |

#### `spec.storage.mode`

| | |
|---|---|
| **Path** | `spec.storage.mode` |
| **Type** | `string` |
| **Description** | Storage mode for persistent data |
| **Default** | `PersistentVolume` |
| **Enum** | `PersistentVolume`, `Local` |

#### `spec.storage.nodeAffinity`

| | |
|---|---|
| **Path** | `spec.storage.nodeAffinity` |
| **Type** | `object` |
| **Description** | Node affinity for local storage mode (optional) |

#### `spec.storage.retentionPolicy`

| | |
|---|---|
| **Path** | `spec.storage.retentionPolicy` |
| **Type** | `string` |
| **Description** | PVC retention policy on node deletion |
| **Default** | `Delete` |
| **Enum** | `Delete`, `Retain` |

#### `spec.storage.size`

| | |
|---|---|
| **Path** | `spec.storage.size` |
| **Type** | `string` |
| **Required** | *(required)* |

#### `spec.storage.snapshotRef`

| | |
|---|---|
| **Path** | `spec.storage.snapshotRef` |
| **Type** | `object` |
| **Description** | Bootstrap this node from a pre-computed snapshot or compressed DB backup. Supports CSI VolumeSnapshot (zero-copy PVC clone) or a compressed archive (.tar.gz / .tar.zst) downloaded by an init container before Stellar Core starts. Reduces catch-up time from days to minutes. |
| **Nullable** | `true` |

##### `spec.storage.snapshotRef.backupUrl`

| | |
|---|---|
| **Path** | `spec.storage.snapshotRef.backupUrl` |
| **Type** | `string` |
| **Description** | URL of a compressed DB backup archive (.tar.gz or .tar.zst). Supported schemes: s3://bucket/path/backup.tar.gz or https://host/path/backup.tar.gz. An init container (snapshot-restore) downloads and extracts the archive into /data before Stellar Core starts. |
| **Nullable** | `true` |

##### `spec.storage.snapshotRef.credentialsSecretRef`

| | |
|---|---|
| **Path** | `spec.storage.snapshotRef.credentialsSecretRef` |
| **Type** | `string` |
| **Description** | Name of a Kubernetes Secret containing credentials for the backup URL. For S3: keys AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_DEFAULT_REGION. For HTTPS: key BEARER_TOKEN. |
| **Nullable** | `true` |

##### `spec.storage.snapshotRef.restoreImage`

| | |
|---|---|
| **Path** | `spec.storage.snapshotRef.restoreImage` |
| **Type** | `string` |
| **Description** | Container image for the restore init container. Defaults to amazon/aws-cli:latest for S3 URLs, alpine:3 for HTTPS. |
| **Nullable** | `true` |

##### `spec.storage.snapshotRef.volumeSnapshotName`

| | |
|---|---|
| **Path** | `spec.storage.snapshotRef.volumeSnapshotName` |
| **Type** | `string` |
| **Description** | Name of an existing VolumeSnapshot (snapshot.storage.k8s.io/v1) in the same namespace. The PVC is provisioned from this snapshot — no init container is needed. |
| **Nullable** | `true` |

##### `spec.storage.snapshotRef.volumeSnapshotNamespace`

| | |
|---|---|
| **Path** | `spec.storage.snapshotRef.volumeSnapshotNamespace` |
| **Type** | `string` |
| **Description** | Optional namespace of the VolumeSnapshot when it lives in a different namespace. Requires CrossNamespaceVolumeDataSource feature gate. |
| **Nullable** | `true` |

#### `spec.storage.storageClass`

| | |
|---|---|
| **Path** | `spec.storage.storageClass` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.strategy`

| | |
|---|---|
| **Path** | `spec.strategy` |
| **Type** | `object` |
| **Description** | Rollout strategy for updates (RollingUpdate or Canary) |
| **Default** | `{'type': 'rollingUpdate'}` |

#### `spec.strategy.canary`

| | |
|---|---|
| **Path** | `spec.strategy.canary` |
| **Type** | `object` |
| **Description** | Configuration for Canary rollout |
| **Nullable** | `true` |

##### `spec.strategy.canary.checkIntervalSeconds`

| | |
|---|---|
| **Path** | `spec.strategy.canary.checkIntervalSeconds` |
| **Type** | `integer` (int32) |
| **Default** | `300` |

##### `spec.strategy.canary.weight`

| | |
|---|---|
| **Path** | `spec.strategy.canary.weight` |
| **Type** | `integer` (int32) |
| **Default** | `10` |

#### `spec.strategy.type`

| | |
|---|---|
| **Path** | `spec.strategy.type` |
| **Type** | `string` |
| **Description** | Rollout strategy type |
| **Required** | *(required)* |
| **Enum** | `rollingUpdate`, `canary` |

### `spec.suspended`

| | |
|---|---|
| **Path** | `spec.suspended` |
| **Type** | `boolean` |
| **Default** | `False` |

### `spec.topologySpreadConstraints`

| | |
|---|---|
| **Path** | `spec.topologySpreadConstraints` |
| **Type** | `array` of `object` |
| **Required** | *(required)* |

### `spec.validatorConfig`

| | |
|---|---|
| **Path** | `spec.validatorConfig` |
| **Type** | `object` |
| **Description** | Validator-specific configuration |
| **Nullable** | `true` |

#### `spec.validatorConfig.catchupComplete`

| | |
|---|---|
| **Path** | `spec.validatorConfig.catchupComplete` |
| **Type** | `boolean` |
| **Description** | Node is in catchup mode (syncing historical data) |
| **Default** | `False` |

#### `spec.validatorConfig.enableHistoryArchive`

| | |
|---|---|
| **Path** | `spec.validatorConfig.enableHistoryArchive` |
| **Type** | `boolean` |
| **Description** | Enable history archive for this validator |
| **Default** | `False` |

#### `spec.validatorConfig.historyArchiveUrls`

| | |
|---|---|
| **Path** | `spec.validatorConfig.historyArchiveUrls` |
| **Type** | `array` of `string` |
| **Description** | History archive URLs to fetch from |

#### `spec.validatorConfig.hsmConfig`

| | |
|---|---|
| **Path** | `spec.validatorConfig.hsmConfig` |
| **Type** | `object` |
| **Description** | Cloud HSM configuration for secure key loading (optional) |
| **Nullable** | `true` |

##### `spec.validatorConfig.hsmConfig.hsmCredentialsSecretRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.hsmConfig.hsmCredentialsSecretRef` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.validatorConfig.hsmConfig.hsmIp`

| | |
|---|---|
| **Path** | `spec.validatorConfig.hsmConfig.hsmIp` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.validatorConfig.hsmConfig.pkcs11LibPath`

| | |
|---|---|
| **Path** | `spec.validatorConfig.hsmConfig.pkcs11LibPath` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.validatorConfig.hsmConfig.provider`

| | |
|---|---|
| **Path** | `spec.validatorConfig.hsmConfig.provider` |
| **Type** | `string` |
| **Description** | Supported HSM Providers |
| **Required** | *(required)* |
| **Enum** | `AWS`, `Azure` |

#### `spec.validatorConfig.keySource`

| | |
|---|---|
| **Path** | `spec.validatorConfig.keySource` |
| **Type** | `string` |
| **Description** | Source of the validator seed (Secret or KMS) |
| **Default** | `secret` |
| **Enum** | `secret`, `kMS` |

#### `spec.validatorConfig.kmsConfig`

| | |
|---|---|
| **Path** | `spec.validatorConfig.kmsConfig` |
| **Type** | `object` |
| **Description** | KMS configuration for fetching the validator seed |
| **Nullable** | `true` |

##### `spec.validatorConfig.kmsConfig.fetcherImage`

| | |
|---|---|
| **Path** | `spec.validatorConfig.kmsConfig.fetcherImage` |
| **Type** | `string` |
| **Nullable** | `true` |

##### `spec.validatorConfig.kmsConfig.keyId`

| | |
|---|---|
| **Path** | `spec.validatorConfig.kmsConfig.keyId` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.validatorConfig.kmsConfig.provider`

| | |
|---|---|
| **Path** | `spec.validatorConfig.kmsConfig.provider` |
| **Type** | `string` |
| **Required** | *(required)* |

##### `spec.validatorConfig.kmsConfig.region`

| | |
|---|---|
| **Path** | `spec.validatorConfig.kmsConfig.region` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `spec.validatorConfig.quorumSet`

| | |
|---|---|
| **Path** | `spec.validatorConfig.quorumSet` |
| **Type** | `string` |
| **Description** | Quorum set configuration as TOML string |
| **Nullable** | `true` |

#### `spec.validatorConfig.seedSecretRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretRef` |
| **Type** | `string` |
| **Description** | Secret name containing the validator seed (key: STELLAR_CORE_SEED) DEPRECATED: Use seed_secret_source for KMS/ESO/CSI-backed secrets in production |
| **Default** | `` |

#### `spec.validatorConfig.seedSecretSource`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource` |
| **Type** | `object` |
| **Description** | Production seed source: ESO (AWS SM / GCP SM / Vault) or CSI Secret Store Driver. Takes precedence over seed_secret_ref when present. |
| **Nullable** | `true` |

##### `spec.validatorConfig.seedSecretSource.csiRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.csiRef` |
| **Type** | `object` |
| **Description** | Secrets Store CSI Driver — **recommended for production**.

Mounts the seed directly from a KMS/Vault into the pod filesystem via a CSI volume.  The seed is never written to etcd.  The controller injects `STELLAR_SEED_FILE` into the container pointing at the mount path; stellar-core reads the key from that file path. |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.csiRef.mountPath`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.csiRef.mountPath` |
| **Type** | `string` |
| **Description** | Directory inside the container where the CSI driver mounts secrets. Defaults to `/mnt/secrets/validator`. |
| **Default** | `/mnt/secrets/validator` |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.csiRef.secretProviderClassName`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.csiRef.secretProviderClassName` |
| **Type** | `string` |
| **Description** | Name of the `SecretProviderClass` CR (from secrets-store.csi.x-k8s.io) that defines which secrets to mount and from which provider. |
| **Required** | *(required)* |

###### `spec.validatorConfig.seedSecretSource.csiRef.seedFileName`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.csiRef.seedFileName` |
| **Type** | `string` |
| **Description** | File name within `mount_path` that contains the seed value. Defaults to `seed`. |
| **Default** | `seed` |
| **Nullable** | `true` |

##### `spec.validatorConfig.seedSecretSource.externalRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef` |
| **Type** | `object` |
| **Description** | External Secrets Operator — **recommended for production**.

The operator creates an `ExternalSecret` CR which causes ESO to pull the seed from AWS Secrets Manager, GCP Secret Manager, HashiCorp Vault, or any other supported backend and materialise it as a Kubernetes Secret in the same namespace.  The seed value is never stored in the CRD itself. |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.externalRef.name`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.name` |
| **Type** | `string` |
| **Description** | Name of the `ExternalSecret` CR the operator will create/manage. Must be unique within the namespace. |
| **Required** | *(required)* |

###### `spec.validatorConfig.seedSecretSource.externalRef.refreshInterval`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.refreshInterval` |
| **Type** | `string` |
| **Description** | How often ESO should re-sync the secret from the remote backend. Kubernetes duration string, e.g. `"1h"`, `"30m"`. Defaults to `"1h"` if not specified. |
| **Default** | `1h` |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.externalRef.remoteKey`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.remoteKey` |
| **Type** | `string` |
| **Description** | Path / identifier of the secret in the remote backend.

Examples: - AWS Secrets Manager: `"prod/stellar/validator-seed"` - GCP Secret Manager: `"projects/MY_PROJECT/secrets/stellar-validator-seed"` - HashiCorp Vault: `"secret/data/stellar/validator"` |
| **Required** | *(required)* |

###### `spec.validatorConfig.seedSecretSource.externalRef.remoteProperty`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.remoteProperty` |
| **Type** | `string` |
| **Description** | Property (field) inside the remote secret to extract.

Required for secrets that store a JSON object (e.g., `{"seed": "S..."}`) and you only want the `seed` value.  Leave empty to use the whole secret value as the seed. |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.externalRef.secretStoreRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.secretStoreRef` |
| **Type** | `object` |
| **Description** | Reference to the `SecretStore` or `ClusterSecretStore` that connects ESO to the remote backend (AWS SM, GCP SM, Vault, etc.). |
| **Required** | *(required)* |

###### `spec.validatorConfig.seedSecretSource.externalRef.secretStoreRef.kind`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.secretStoreRef.kind` |
| **Type** | `string` |
| **Description** | Kind of the store resource.

- `"SecretStore"` — namespaced store (only works within the same namespace) - `"ClusterSecretStore"` — cluster-wide store (recommended for production) |
| **Default** | `ClusterSecretStore` |

###### `spec.validatorConfig.seedSecretSource.externalRef.secretStoreRef.name`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef.secretStoreRef.name` |
| **Type** | `string` |
| **Description** | Name of the `SecretStore` / `ClusterSecretStore` resource. |
| **Required** | *(required)* |

##### `spec.validatorConfig.seedSecretSource.localRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.localRef` |
| **Type** | `object` |
| **Description** | Plain Kubernetes Secret — **development only**.

Points to an existing `Secret` in the same namespace.  The secret must contain the key specified in `key` (defaults to `STELLAR_CORE_SEED`). |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.localRef.key`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.localRef.key` |
| **Type** | `string` |
| **Description** | Key within the secret that holds the seed value. Defaults to `STELLAR_CORE_SEED` if not specified. |
| **Default** | `STELLAR_CORE_SEED` |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.localRef.name`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.localRef.name` |
| **Type** | `string` |
| **Description** | Name of the `Secret` in the same namespace. |
| **Required** | *(required)* |

##### `spec.validatorConfig.seedSecretSource.vaultRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef` |
| **Type** | `object` |
| **Description** | HashiCorp Vault via the **Vault Agent Injector** (init + sidecar).

Requires the Vault Agent Injector mutating webhook in the cluster. The operator sets standard `vault.hashicorp.com/*` pod annotations; the injector adds the Vault Agent containers and renders the secret file under `/vault/secrets/`. |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations` |
| **Type** | `array` of `object` |
| **Description** | Additional `vault.hashicorp.com/*` or other pod annotations to merge. |

###### `spec.validatorConfig.seedSecretSource.vaultRef.restartOnSecretRotation`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.restartOnSecretRotation` |
| **Type** | `boolean` |
| **Description** | When true, the operator compares Vault secret-version annotations on pods and rolls the StatefulSet when the version changes after sync. |
| **Default** | `False` |

###### `spec.validatorConfig.seedSecretSource.vaultRef.role`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.role` |
| **Type** | `string` |
| **Description** | Vault Kubernetes auth role bound to this pod's ServiceAccount. |
| **Required** | *(required)* |

###### `spec.validatorConfig.seedSecretSource.vaultRef.secretFileName`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.secretFileName` |
| **Type** | `string` |
| **Description** | Base file name rendered under `/vault/secrets/` (default `stellar-seed`). |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.vaultRef.secretKey`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.secretKey` |
| **Type** | `string` |
| **Description** | JSON field under `.Data.data` for KV v2 (default `seed`). Ignored if `template` is set. |
| **Nullable** | `true` |

###### `spec.validatorConfig.seedSecretSource.vaultRef.secretPath`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.secretPath` |
| **Type** | `string` |
| **Description** | Path passed to `vault.hashicorp.com/agent-inject-secret-<file>` (KV v1/v2 path as in Vault). |
| **Required** | *(required)* |

###### `spec.validatorConfig.seedSecretSource.vaultRef.template`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.template` |
| **Type** | `string` |
| **Description** | Custom Agent template; when set, overrides the default KV v2 template. |
| **Nullable** | `true` |

#### `spec.validatorConfig.vlSource`

| | |
|---|---|
| **Path** | `spec.validatorConfig.vlSource` |
| **Type** | `string` |
| **Description** | Trusted source for Validator Selection List (VSL) |
| **Nullable** | `true` |

### `spec.version`

| | |
|---|---|
| **Path** | `spec.version` |
| **Type** | `string` |
| **Required** | *(required)* |

### `spec.vpaConfig`

| | |
|---|---|
| **Path** | `spec.vpaConfig` |
| **Type** | `object` |
| **Description** | VPA configuration |
| **Nullable** | `true` |

#### `spec.vpaConfig.containerPolicies`

| | |
|---|---|
| **Path** | `spec.vpaConfig.containerPolicies` |
| **Type** | `array` of `object` |

#### `spec.vpaConfig.updateMode`

| | |
|---|---|
| **Path** | `spec.vpaConfig.updateMode` |
| **Type** | `string` |
| **Description** | VPA update mode |
| **Default** | `Initial` |
| **Enum** | `Initial`, `Auto` |

## Status Fields


### `status.bgpStatus`

| | |
|---|---|
| **Path** | `status.bgpStatus` |
| **Type** | `object` |
| **Description** | BGP advertisement status (when using BGP mode) |
| **Nullable** | `true` |

#### `status.bgpStatus.activePeers`

| | |
|---|---|
| **Path** | `status.bgpStatus.activePeers` |
| **Type** | `integer` (int32) |
| **Description** | Number of active BGP peers |
| **Required** | *(required)* |

#### `status.bgpStatus.advertisedPrefixes`

| | |
|---|---|
| **Path** | `status.bgpStatus.advertisedPrefixes` |
| **Type** | `array` of `string` |
| **Description** | Advertised IP prefixes |

#### `status.bgpStatus.lastUpdate`

| | |
|---|---|
| **Path** | `status.bgpStatus.lastUpdate` |
| **Type** | `string` |
| **Description** | Last BGP update time |
| **Nullable** | `true` |

#### `status.bgpStatus.sessionsEstablished`

| | |
|---|---|
| **Path** | `status.bgpStatus.sessionsEstablished` |
| **Type** | `boolean` |
| **Description** | Whether BGP sessions are established |
| **Required** | *(required)* |

### `status.canaryReadyReplicas`

| | |
|---|---|
| **Path** | `status.canaryReadyReplicas` |
| **Type** | `integer` (int32) |
| **Description** | Current number of ready canary replicas (for canary deployments) |
| **Default** | `0` |

### `status.canaryStartTime`

| | |
|---|---|
| **Path** | `status.canaryStartTime` |
| **Type** | `string` |
| **Description** | Timestamp when the canary was created (RFC3339) |
| **Nullable** | `true` |

### `status.canaryVersion`

| | |
|---|---|
| **Path** | `status.canaryVersion` |
| **Type** | `string` |
| **Description** | Version deployed in the canary deployment (if active) |
| **Nullable** | `true` |

### `status.conditions`

| | |
|---|---|
| **Path** | `status.conditions` |
| **Type** | `array` of `object` |
| **Description** | Readiness conditions following Kubernetes conventions

Standard conditions include: - Ready: True when all sub-resources are healthy and the node is operational - Progressing: True when the node is being created, updated, or syncing - Degraded: True when the node is operational but experiencing issues |

### `status.drStatus`

| | |
|---|---|
| **Path** | `status.drStatus` |
| **Type** | `object` |
| **Description** | Status of the cross-region disaster recovery setup (if enabled) |
| **Nullable** | `true` |

#### `status.drStatus.currentRole`

| | |
|---|---|
| **Path** | `status.drStatus.currentRole` |
| **Type** | `string` |
| **Description** | Role of a node in a DR configuration |
| **Nullable** | `true` |
| **Enum** | `primary`, `standby` |

#### `status.drStatus.failoverActive`

| | |
|---|---|
| **Path** | `status.drStatus.failoverActive` |
| **Type** | `boolean` |
| **Required** | *(required)* |

#### `status.drStatus.lastDrillResult`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult` |
| **Type** | `object` |
| **Description** | Result of a DR drill execution |
| **Nullable** | `true` |

##### `status.drStatus.lastDrillResult.applicationAvailability`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.applicationAvailability` |
| **Type** | `boolean` |
| **Description** | Whether application remained available during drill |
| **Required** | *(required)* |

##### `status.drStatus.lastDrillResult.completedAt`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.completedAt` |
| **Type** | `string` |
| **Description** | Timestamp when drill completed |
| **Nullable** | `true` |

##### `status.drStatus.lastDrillResult.message`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.message` |
| **Type** | `string` |
| **Description** | Human-readable message about drill result |
| **Required** | *(required)* |

##### `status.drStatus.lastDrillResult.standbyTakeoverSuccess`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.standbyTakeoverSuccess` |
| **Type** | `boolean` |
| **Description** | Whether standby successfully took over |
| **Required** | *(required)* |

##### `status.drStatus.lastDrillResult.startedAt`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.startedAt` |
| **Type** | `string` |
| **Description** | Timestamp when drill started |
| **Required** | *(required)* |

##### `status.drStatus.lastDrillResult.status`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.status` |
| **Type** | `string` |
| **Description** | Drill execution status |
| **Required** | *(required)* |
| **Enum** | `pending`, `running`, `success`, `failed`, `rolledback` |

##### `status.drStatus.lastDrillResult.timeToRecoveryMs`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillResult.timeToRecoveryMs` |
| **Type** | `integer` (uint64) |
| **Description** | Time to recovery in milliseconds |
| **Nullable** | `true` |

#### `status.drStatus.lastDrillTime`

| | |
|---|---|
| **Path** | `status.drStatus.lastDrillTime` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `status.drStatus.lastPeerContact`

| | |
|---|---|
| **Path** | `status.drStatus.lastPeerContact` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `status.drStatus.peerHealth`

| | |
|---|---|
| **Path** | `status.drStatus.peerHealth` |
| **Type** | `string` |
| **Nullable** | `true` |

#### `status.drStatus.syncLag`

| | |
|---|---|
| **Path** | `status.drStatus.syncLag` |
| **Type** | `integer` (uint64) |
| **Nullable** | `true` |

### `status.endpoint`

| | |
|---|---|
| **Path** | `status.endpoint` |
| **Type** | `string` |
| **Description** | Endpoint where the node is accessible (Service ClusterIP or external) |
| **Nullable** | `true` |

### `status.externalIp`

| | |
|---|---|
| **Path** | `status.externalIp` |
| **Type** | `string` |
| **Description** | External load balancer IP assigned by MetalLB |
| **Nullable** | `true` |

### `status.forensicSnapshotPhase`

| | |
|---|---|
| **Path** | `status.forensicSnapshotPhase` |
| **Type** | `string` |
| **Description** | Phase of the last forensic snapshot request (`Pending`, `Capturing`, `Complete`, `Failed`). |
| **Nullable** | `true` |

### `status.labelPropagationStatus`

| | |
|---|---|
| **Path** | `status.labelPropagationStatus` |
| **Type** | `string` |
| **Description** | Result of the last label propagation pass. One of "Synced", "Partial", "Failed" |
| **Nullable** | `true` |

### `status.lastMigratedVersion`

| | |
|---|---|
| **Path** | `status.lastMigratedVersion` |
| **Type** | `string` |
| **Description** | Version of the database schema after last successful migration |
| **Nullable** | `true` |

### `status.ledgerSequence`

| | |
|---|---|
| **Path** | `status.ledgerSequence` |
| **Type** | `integer` (uint64) |
| **Description** | For validators: current ledger sequence number |
| **Nullable** | `true` |

### `status.ledgerUpdatedAt`

| | |
|---|---|
| **Path** | `status.ledgerUpdatedAt` |
| **Type** | `string` |
| **Description** | Timestamp of the last ledger update (RFC3339) |
| **Nullable** | `true` |

### `status.message`

| | |
|---|---|
| **Path** | `status.message` |
| **Type** | `string` |
| **Description** | Human-readable message about current state |
| **Nullable** | `true` |

### `status.observedGeneration`

| | |
|---|---|
| **Path** | `status.observedGeneration` |
| **Type** | `integer` (int64) |
| **Description** | Observed generation for status sync detection |
| **Nullable** | `true` |

### `status.phase`

| | |
|---|---|
| **Path** | `status.phase` |
| **Type** | `string` |
| **Description** | Current phase of the node lifecycle (Pending, Creating, Running, Syncing, Ready, Failed, Degraded, Remediating, Terminating)

DEPRECATED: Use the conditions array instead. This field is maintained for backward compatibility and will be removed in a future version. The phase is now derived from the conditions. |
| **Required** | *(required)* |

### `status.quorumAnalysisTimestamp`

| | |
|---|---|
| **Path** | `status.quorumAnalysisTimestamp` |
| **Type** | `string` |
| **Description** | Timestamp of last quorum analysis (RFC3339) |
| **Nullable** | `true` |

### `status.quorumFragility`

| | |
|---|---|
| **Path** | `status.quorumFragility` |
| **Type** | `number` (double) |
| **Description** | Quorum fragility score (0.0 = resilient, 1.0 = fragile) Only populated for validator nodes |
| **Nullable** | `true` |

### `status.readyReplicas`

| | |
|---|---|
| **Path** | `status.readyReplicas` |
| **Type** | `integer` (int32) |
| **Description** | Current number of ready replicas |
| **Default** | `0` |

### `status.replicas`

| | |
|---|---|
| **Path** | `status.replicas` |
| **Type** | `integer` (int32) |
| **Description** | Total number of desired replicas |
| **Default** | `0` |

### `status.snapshotBootstrap`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap` |
| **Type** | `object` |
| **Description** | Bootstrap status when the node was started from a snapshot or compressed backup. Tracks the restore phase and time-to-sync for observability. A secondsToSync value ≤ 600 satisfies the "synced within 10 minutes" acceptance criterion. |
| **Nullable** | `true` |

#### `status.snapshotBootstrap.message`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.message` |
| **Type** | `string` |
| **Description** | Human-readable message about the current bootstrap state. |
| **Nullable** | `true` |

#### `status.snapshotBootstrap.phase`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.phase` |
| **Type** | `string` |
| **Description** | Current phase of the bootstrap operation. One of: Pending, Restoring, Restored, Syncing, Synced, Failed |
| **Required** | *(required)* |

#### `status.snapshotBootstrap.restoreCompletedAt`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.restoreCompletedAt` |
| **Type** | `string` |
| **Description** | RFC3339 timestamp when the restore init container completed successfully. |
| **Nullable** | `true` |

#### `status.snapshotBootstrap.restoreStartedAt`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.restoreStartedAt` |
| **Type** | `string` |
| **Description** | RFC3339 timestamp when the restore init container started. |
| **Nullable** | `true` |

#### `status.snapshotBootstrap.secondsToSync`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.secondsToSync` |
| **Type** | `integer` (uint64) |
| **Description** | Elapsed seconds from restore completion to first Synced state. A value ≤ 600 satisfies the "synced within 10 minutes" acceptance criterion. |
| **Nullable** | `true` |

#### `status.snapshotBootstrap.source`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.source` |
| **Type** | `string` |
| **Description** | Source used for bootstrap (VolumeSnapshot name or backup URL). |
| **Nullable** | `true` |

#### `status.snapshotBootstrap.syncedAt`

| | |
|---|---|
| **Path** | `status.snapshotBootstrap.syncedAt` |
| **Type** | `string` |
| **Description** | RFC3339 timestamp when the node first reached Synced state after bootstrap. |
| **Nullable** | `true` |

### `status.vaultObservedSecretVersion`

| | |
|---|---|
| **Path** | `status.vaultObservedSecretVersion` |
| **Type** | `string` |
| **Description** | Last observed Vault secret version annotation (for rotation-driven rollouts). |
| **Nullable** | `true` |
