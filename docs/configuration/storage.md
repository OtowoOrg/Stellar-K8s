# Storage Configuration

Storage fields for `StellarNode` resources are listed in the [CRD API reference](../api-reference.md).

For persistent volume sizing and operations, see the [Validator deployment guide](../deployment-guides/validator.md). For CSI snapshots and restoring validator data, see [Volume Snapshots](../volume-snapshots.md).

!!! note "Fresh volumes need schema initialization"
    A brand-new data volume must be initialized with `stellar-core new-db` before
    the validator can boot on it. See [Database Schema Initialization](../operations/db-schema-init.md)
    for when this is required, the procedure, and the failure symptom when skipped.