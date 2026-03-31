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
| `Replicas` | `integer` | `.spec.replicas` |
| `Phase` | `string` | `.status.phase` |
| `Age` | `date` | `.metadata.creationTimestamp` |

## Spec Fields

Fields marked *(required)* must be present in every `StellarNode` manifest.

### `horizonConfig`

| | |
|---|---|
| **Path** | `spec.horizonConfig` |
| **Type** | `object` |

#### `horizonConfig.databaseSecretRef`

| | |
|---|---|
| **Path** | `spec.horizonConfig.databaseSecretRef` |
| **Type** | `string` |

#### `horizonConfig.enableExperimentalIngestion`

| | |
|---|---|
| **Path** | `spec.horizonConfig.enableExperimentalIngestion` |
| **Type** | `boolean` |

#### `horizonConfig.enableIngest`

| | |
|---|---|
| **Path** | `spec.horizonConfig.enableIngest` |
| **Type** | `boolean` |

#### `horizonConfig.ingestWorkers`

| | |
|---|---|
| **Path** | `spec.horizonConfig.ingestWorkers` |
| **Type** | `integer` |

#### `horizonConfig.stellarCoreUrl`

| | |
|---|---|
| **Path** | `spec.horizonConfig.stellarCoreUrl` |
| **Type** | `string` |

### `network` *(required)*

| | |
|---|---|
| **Path** | `spec.network` |
| **Type** | `string` |

Target Stellar network

### `nodeType` *(required)*

| | |
|---|---|
| **Path** | `spec.nodeType` |
| **Type** | `string (enum)` |
| **Constraint** | one of: `Validator`, `Horizon`, `SorobanRpc` |

Type of Stellar node to deploy

### `replicas`

| | |
|---|---|
| **Path** | `spec.replicas` |
| **Type** | `integer` |
| **Default** | `1` |
| **Constraint** | min: `0` |

Number of replicas (RPC nodes only)

### `resources`

| | |
|---|---|
| **Path** | `spec.resources` |
| **Type** | `object` |

#### `resources.limits`

| | |
|---|---|
| **Path** | `spec.resources.limits` |
| **Type** | `object` |

##### `resources.limits.cpu`

| | |
|---|---|
| **Path** | `spec.resources.limits.cpu` |
| **Type** | `string` |

##### `resources.limits.memory`

| | |
|---|---|
| **Path** | `spec.resources.limits.memory` |
| **Type** | `string` |

#### `resources.requests`

| | |
|---|---|
| **Path** | `spec.resources.requests` |
| **Type** | `object` |

##### `resources.requests.cpu`

| | |
|---|---|
| **Path** | `spec.resources.requests.cpu` |
| **Type** | `string` |

##### `resources.requests.memory`

| | |
|---|---|
| **Path** | `spec.resources.requests.memory` |
| **Type** | `string` |

### `restoreFromSnapshot`

| | |
|---|---|
| **Path** | `spec.restoreFromSnapshot` |
| **Type** | `object` |

Bootstrap this node from an existing VolumeSnapshot (Validator only). PVC is created from the snapshot.

#### `restoreFromSnapshot.namespace`

| | |
|---|---|
| **Path** | `spec.restoreFromSnapshot.namespace` |
| **Type** | `string` |

Optional namespace of the VolumeSnapshot (CrossNamespaceVolumeDataSource).

#### `restoreFromSnapshot.volumeSnapshotName`

| | |
|---|---|
| **Path** | `spec.restoreFromSnapshot.volumeSnapshotName` |
| **Type** | `string` |

Name of the VolumeSnapshot to restore from (same namespace as StellarNode).

### `serviceConfig`

| | |
|---|---|
| **Path** | `spec.serviceConfig` |
| **Type** | `object` |

#### `serviceConfig.httpNodePort`

| | |
|---|---|
| **Path** | `spec.serviceConfig.httpNodePort` |
| **Type** | `integer` |
| **Constraint** | min: `30000` |
| **Constraint** | max: `32767` |

NodePort for HTTP API access

#### `serviceConfig.peerNodePort`

| | |
|---|---|
| **Path** | `spec.serviceConfig.peerNodePort` |
| **Type** | `integer` |
| **Constraint** | min: `30000` |
| **Constraint** | max: `32767` |

NodePort for peer connections (validators only)

### `snapshotSchedule`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule` |
| **Type** | `object` |

Schedule and options for CSI VolumeSnapshots of the node data PVC (Validator only).

#### `snapshotSchedule.flushBeforeSnapshot`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.flushBeforeSnapshot` |
| **Type** | `boolean` |
| **Default** | `false` |

If true, attempt to flush/lock the Stellar DB briefly before creating the snapshot.

#### `snapshotSchedule.retentionCount`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.retentionCount` |
| **Type** | `integer (int32)` |
| **Default** | `0` |

Max snapshots to retain per node. 0 means no limit.

#### `snapshotSchedule.schedule`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.schedule` |
| **Type** | `string` |

Cron expression for scheduled snapshots (e.g. "0 2 * * *" for daily at 2 AM). If unset, snapshots only on annotation stellar.org/request-snapshot=true.

#### `snapshotSchedule.volumeSnapshotClassName`

| | |
|---|---|
| **Path** | `spec.snapshotSchedule.volumeSnapshotClassName` |
| **Type** | `string` |

VolumeSnapshotClass name. If unset, default class for the driver is used.

### `sorobanConfig`

| | |
|---|---|
| **Path** | `spec.sorobanConfig` |
| **Type** | `object` |

#### `sorobanConfig.captiveCoreConfig`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.captiveCoreConfig` |
| **Type** | `string` |

#### `sorobanConfig.enablePreflight`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.enablePreflight` |
| **Type** | `boolean` |

#### `sorobanConfig.maxEventsPerRequest`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.maxEventsPerRequest` |
| **Type** | `integer` |

#### `sorobanConfig.stellarCoreUrl`

| | |
|---|---|
| **Path** | `spec.sorobanConfig.stellarCoreUrl` |
| **Type** | `string` |

### `storage`

| | |
|---|---|
| **Path** | `spec.storage` |
| **Type** | `object` |

#### `storage.annotations`

| | |
|---|---|
| **Path** | `spec.storage.annotations` |
| **Type** | `map[string]string` |
| **Nullable** | `true` |

Optional annotations to apply to the PersistentVolumeClaim (useful for storage-class specific parameters like volumeBindingMode)

#### `storage.retentionPolicy`

| | |
|---|---|
| **Path** | `spec.storage.retentionPolicy` |
| **Type** | `string (enum)` |
| **Default** | `Delete` |
| **Constraint** | one of: `Delete`, `Retain` |

#### `storage.size`

| | |
|---|---|
| **Path** | `spec.storage.size` |
| **Type** | `string` |

#### `storage.storageClass`

| | |
|---|---|
| **Path** | `spec.storage.storageClass` |
| **Type** | `string` |

### `suspended`

| | |
|---|---|
| **Path** | `spec.suspended` |
| **Type** | `boolean` |
| **Default** | `false` |

Suspend the node without deleting resources

### `validatorConfig`

| | |
|---|---|
| **Path** | `spec.validatorConfig` |
| **Type** | `object` |

#### `validatorConfig.catchupComplete`

| | |
|---|---|
| **Path** | `spec.validatorConfig.catchupComplete` |
| **Type** | `boolean` |

#### `validatorConfig.enableHistoryArchive`

| | |
|---|---|
| **Path** | `spec.validatorConfig.enableHistoryArchive` |
| **Type** | `boolean` |

#### `validatorConfig.historyArchiveUrls`

| | |
|---|---|
| **Path** | `spec.validatorConfig.historyArchiveUrls` |
| **Type** | `[]string` |

#### `validatorConfig.quorumSet`

| | |
|---|---|
| **Path** | `spec.validatorConfig.quorumSet` |
| **Type** | `string` |

#### `validatorConfig.seedSecretRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretRef` |
| **Type** | `string` |

#### `validatorConfig.seedSecretSource`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource` |
| **Type** | `object` |

Typed seed source; exactly one of localRef, externalRef, csiRef, vaultRef.

##### `validatorConfig.seedSecretSource.csiRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.csiRef` |
| **Type** | `object` |

##### `validatorConfig.seedSecretSource.externalRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.externalRef` |
| **Type** | `object` |

##### `validatorConfig.seedSecretSource.localRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.localRef` |
| **Type** | `object` |

##### `validatorConfig.seedSecretSource.vaultRef`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef` |
| **Type** | `object` |

###### `validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations` |
| **Type** | `[]object` |

###### Items of `extraPodAnnotations`

###### `validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations[].name`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations[].name` |
| **Type** | `string` |

###### `validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations[].value`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.extraPodAnnotations[].value` |
| **Type** | `string` |

###### `validatorConfig.seedSecretSource.vaultRef.restartOnSecretRotation`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.restartOnSecretRotation` |
| **Type** | `boolean` |

###### `validatorConfig.seedSecretSource.vaultRef.role`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.role` |
| **Type** | `string` |

###### `validatorConfig.seedSecretSource.vaultRef.secretFileName`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.secretFileName` |
| **Type** | `string` |
| **Nullable** | `true` |

###### `validatorConfig.seedSecretSource.vaultRef.secretKey`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.secretKey` |
| **Type** | `string` |
| **Nullable** | `true` |

###### `validatorConfig.seedSecretSource.vaultRef.secretPath`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.secretPath` |
| **Type** | `string` |

###### `validatorConfig.seedSecretSource.vaultRef.template`

| | |
|---|---|
| **Path** | `spec.validatorConfig.seedSecretSource.vaultRef.template` |
| **Type** | `string` |
| **Nullable** | `true` |

### `version` *(required)*

| | |
|---|---|
| **Path** | `spec.version` |
| **Type** | `string` |

Container image version

---

## Status Fields

Status fields are written by the operator and are read-only for users.

### `conditions`

| | |
|---|---|
| **Path** | `status.conditions` |
| **Type** | `[]object` |

#### Items of `conditions`

#### `status.conditions[].lastTransitionTime`

| | |
|---|---|
| **Path** | `status.conditions[].lastTransitionTime` |
| **Type** | `string` |

#### `status.conditions[].message`

| | |
|---|---|
| **Path** | `status.conditions[].message` |
| **Type** | `string` |

#### `status.conditions[].reason`

| | |
|---|---|
| **Path** | `status.conditions[].reason` |
| **Type** | `string` |

#### `status.conditions[].status`

| | |
|---|---|
| **Path** | `status.conditions[].status` |
| **Type** | `string` |

#### `status.conditions[].type`

| | |
|---|---|
| **Path** | `status.conditions[].type` |
| **Type** | `string` |

### `endpoint`

| | |
|---|---|
| **Path** | `status.endpoint` |
| **Type** | `string` |

### `forensicSnapshotPhase`

| | |
|---|---|
| **Path** | `status.forensicSnapshotPhase` |
| **Type** | `string` |

Phase of forensic snapshot (Capturing, Complete, Failed).

### `ledgerSequence`

| | |
|---|---|
| **Path** | `status.ledgerSequence` |
| **Type** | `integer (int64)` |

### `message`

| | |
|---|---|
| **Path** | `status.message` |
| **Type** | `string` |

### `observedGeneration`

| | |
|---|---|
| **Path** | `status.observedGeneration` |
| **Type** | `integer (int64)` |

### `phase`

| | |
|---|---|
| **Path** | `status.phase` |
| **Type** | `string` |

### `readyReplicas`

| | |
|---|---|
| **Path** | `status.readyReplicas` |
| **Type** | `integer` |

### `replicas`

| | |
|---|---|
| **Path** | `status.replicas` |
| **Type** | `integer` |

### `vaultObservedSecretVersion`

| | |
|---|---|
| **Path** | `status.vaultObservedSecretVersion` |
| **Type** | `string` |

Last Vault secret version observed for rotation-driven rollouts.

---

## Example Manifest

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-validator
  namespace: stellar-system
spec:
  nodeType: Validator
  network: Testnet
  version: v21.0.0
  replicas: 1
  resources:
    requests:
      cpu: 500m
      memory: 1Gi
    limits:
      cpu: "2"
      memory: 4Gi
  storage:
    storageClass: standard
    size: 100Gi
    retentionPolicy: Delete
  validatorConfig:
    seedSecretRef: my-validator-seed
    enableHistoryArchive: true
    historyArchiveUrls:
      - https://history.stellar.org/prd/core-testnet/core_testnet_001
```
