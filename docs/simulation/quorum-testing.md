# Stellar-K8s Simulation Environment for Quorum Testing

## Overview
A 'Shadow Cluster' feature that allows operators to test quorum changes or network upgrades against a simulated environment before rolling them out to production.

## Implementation Details
- Deploys a parallel 'Shadow' cluster inside a dedicated namespace (or locally via Kind/K3d).
- Replays recent mainnet traffic (in read-only mode, disabling active participation in consensus) to the shadow nodes.
- Validates that the proposed target configuration reaches consensus under the simulated workload.
- Extracts telemetry to evaluate and report on the overall 'Quorum Safety Margin'.
