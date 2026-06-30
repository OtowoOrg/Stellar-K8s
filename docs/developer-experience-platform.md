# Developer Experience Platform

Local development environment for Stellar-K8s with hot reload, port forwarding,
and IDE integration.

## Quick Start (< 1 hour)

```bash
# Option A: Tilt (recommended)
tilt up

# Option B: Skaffold
skaffold dev --profile dev

# Option C: Make + kind
make quickstart
make compose-dev
```

## Hot Reload

Tilt and `make run-dev` use `cargo watch` for sub-5-second iteration cycles.

## Port Forwarding

| Service | Local Port |
|---------|------------|
| Operator REST API | 8080 |
| Prometheus metrics | 9090 |
| Horizon (sample) | 8000 |

## Log Streaming

```bash
kubectl stellar logs <node-name> -f
stellar-sidecar --stream
```

## Resource Templates

`dev/templates/` contains 12 quick-start templates for common workflows.

## VS Code Extension

Install from `tools/vscode-stellar/` for remote debugging and log streaming.

## Remote Debugging

Attach VS Code to the operator pod:

```json
{
  "name": "Attach to stellar-operator",
  "type": "lldb",
  "request": "attach",
  "pid": "${command:pickProcess}"
}
```
