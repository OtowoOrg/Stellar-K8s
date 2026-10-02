# Optimizing Persistent Volume Claims (PVCs) for Captive Core Sync

## Context & Impact
Stellar's ledger processes massive amounts of state data. Utilizing improper physical storage solutions inside Kubernetes leads to node synchronization failures and degraded APIs. This guide optimizes Kubernetes storage classes specifically for Stellar, ensuring maximum performance and reliability.

## Stateless Containers vs. Stateful Blockchain Pods
Kubernetes was originally designed for stateless workloads where containers can be spun up, destroyed, and replaced without data loss. In a stateless architecture, the container image and environment variables are sufficient to maintain the application's state.

However, blockchain nodes (like Stellar Core and Horizon) are **stateful**. They maintain a local copy of the distributed ledger. If a Stellar pod is destroyed and recreated, it must either re-sync from the network (which can take days) or reconnect to its existing local storage. 

This is why **external physical storage provisioning via the Container Storage Interface (CSI)** is critical for data survival. By decoupling the storage lifecycle from the pod lifecycle through Persistent Volumes (PVs) and Persistent Volume Claims (PVCs), we ensure that a node's ledger data survives pod restarts, rescheduling, and cluster upgrades.

## Configuring StorageClasses for Maximum IOPS
Stellar validators, particularly during Captive Core sync, require extremely high IOPS (Input/Output Operations Per Second). Standard network-attached storage (NAS) often introduces latency that causes nodes to fall out of sync with the network.

To maximize performance, we must configure Kubernetes `StorageClass` objects to utilize local **NVMe SSDs** rather than standard network volumes.

### High-IOPS StorageClass Example (AWS EBS io2)
For cloud environments, utilize provisioned IOPS SSDs. In AWS, this means using `io2` Block Express volumes.

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-high-iops
provisioner: ebs.csi.aws.com
volumeBindingMode: WaitForFirstConsumer
allowVolumeExpansion: true
parameters:
  type: io2
  iopsPerGB: "50"
  fsType: ext4
```

### Local NVMe StorageClass Example
For bare-metal or instances with local NVMe drives, use the local provisioner.

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-local-nvme
provisioner: kubernetes.io/no-provisioner
volumeBindingMode: WaitForFirstConsumer
```

## Horizon Archival Nodes vs. Active Validators
The storage requirements for a Stellar node depend heavily on its role.

### Active Validators
* **Requirement**: Extremely high IOPS, low latency.
* **Capacity**: Moderate (usually enough for the current state and a small history buffer).
* **StorageClass**: `stellar-high-iops` (e.g., io2, local NVMe).
* **Cost focus**: Paying for performance (IOPS).

### Horizon Archival Nodes
* **Requirement**: High capacity, moderate IOPS.
* **Capacity**: Very large (often several Terabytes to store full ledger history).
* **StorageClass**: Standard SSD or specialized capacity-optimized volumes (e.g., AWS `gp3` with baseline IOPS).
* **Cost focus**: Paying for capacity (GB).

## Linking StellarNode CRDs to High-Performance Volumes
When deploying a Stellar node using the Stellar operator (via the `StellarNode` Custom Resource Definition), you can link the node to the optimized `StorageClass`.

Here is an example demonstrating how to link a `StellarNode` to a high-performance local volume using the storage spec block:

```yaml
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarNode
metadata:
  name: validator-node
  namespace: stellar
spec:
  network: "public"
  nodeType: "validator"
  storage:
    size: "100Gi"
    storageClassName: "stellar-high-iops"
    retentionPolicy: Retain
```
Setting `retentionPolicy: Retain` ensures that the underlying physical volume is not deleted if the `StellarNode` resource is removed, providing an extra layer of data safety.
