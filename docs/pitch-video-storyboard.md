# Pitch Video Storyboard

**Summary:** A two-minute pitch video showing a platform engineer replacing hand-run Stellar nodes with one Helm install and a `StellarNode` manifest, then watching the operator reconcile, sync and report health. It uses only shipped capabilities; roadmap items are not shown.

| # | Scene | What is on screen | Source |
| - | ----- | ----------------- | ------ |
| 1 | The problem | Hand-maintained Stellar Core, Horizon and Soroban RPC deployments drifting apart | [Problem](pitch-storyboards.md#problem) |
| 2 | Install | `helm install` of `charts/stellar-operator` | [Installation](getting-started/installation.md) |
| 3 | Declare a node | Applying a Testnet validator `StellarNode` | [Journey A](pitch-storyboards.md#user-journeys) |
| 4 | Reconcile and sync | `kubectl get stellarnodes` until the node reports Ready | [Implemented today](pitch-storyboards.md#implemented-today) |
| 5 | Operate | `kubectl-stellar` plugin and Prometheus metrics | [Implemented today](pitch-storyboards.md#implemented-today) |
| 6 | Call to action | Documentation site and CONTRIBUTING | [Journey C](pitch-storyboards.md#user-journeys) |

The full narrative, users and journeys are in [PITCH Storyboards](pitch-storyboards.md). Update this note if the video script changes.
