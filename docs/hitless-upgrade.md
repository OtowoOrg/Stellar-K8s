# Hitless Upgrade for Stellar Core Peer Connections

## Overview

This document describes the design and implementation of hot-upgrade support
for Stellar Core containers, enabling zero-interruption upgrades that preserve
active TCP peer connections.

## Feasibility Study: FD Passing Between Containers

### Background

Every time Stellar Core restarts, it drops all active TCP peer connections.
Peers must then re-discover and re-handshake, causing a brief period of
reduced connectivity. For Tier-1 validators, this interruption can affect
consensus participation.

### Approach: Socket Handoff via Unix Domain Socket

The standard Linux mechanism for passing open file descriptors between
processes is `SCM_RIGHTS` over a Unix domain socket. This allows a running
process to transfer ownership of an open TCP socket to a new process before
the old process exits.

**Feasibility within Kubernetes:**

| Constraint | Assessment |
|---|---|
| Shared Unix socket between containers | ✅ Feasible via shared `emptyDir` volume |
| `SCM_RIGHTS` in containers | ✅ Supported; no special capabilities needed |
| Stellar Core support for FD injection | ⚠️ Requires upstream patch or wrapper |
| Zero consensus message loss | ✅ Achievable with careful sequencing |

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        Pod                              │
│                                                         │
│  ┌──────────────────┐    Unix socket    ┌────────────┐  │
│  │  stellar-core    │◄─────────────────►│  handoff   │  │
│  │  (old version)   │   /handoff/sock   │  sidecar   │  │
│  └──────────────────┘                   └────────────┘  │
│                                               │         │
│  ┌──────────────────┐                         │         │
│  │  stellar-core    │◄────────────────────────┘         │
│  │  (new version)   │   receives FDs via SCM_RIGHTS      │
│  └──────────────────┘                                   │
└─────────────────────────────────────────────────────────┘
```

### Handoff Sidecar

The `stellar-handoff` sidecar container:

1. **Listens** on a Unix domain socket at `/handoff/sock` (shared `emptyDir` volume).
2. **Receives** a `HANDOFF_REQUEST` signal from the old Stellar Core process.
3. **Collects** open peer socket FDs from the old process via `/proc/<pid>/fd`.
4. **Transfers** the FDs to the new Stellar Core process using `SCM_RIGHTS`.
5. **Signals** the new process that FD injection is complete.

### Upgrade Sequence

```
1. Operator detects new version in StellarNodeSpec
2. Operator annotates pod: stellar.org/hitless-upgrade=pending
3. Handoff sidecar wakes up, signals old stellar-core to prepare
4. Old stellar-core: stops accepting new connections, queues messages
5. Handoff sidecar: collects peer socket FDs via SCM_RIGHTS
6. Operator: starts new stellar-core container (init container pattern)
7. Handoff sidecar: injects FDs into new process
8. New stellar-core: resumes processing with inherited connections
9. Old stellar-core: exits cleanly
10. Operator: removes annotation, updates status
```

### Consensus Safety

- The old process **queues** (does not drop) incoming SCP messages during the
  handoff window (configurable, default 5 seconds).
- The new process **replays** queued messages after FD injection.
- If the handoff window expires without successful FD transfer, the operator
  falls back to a standard rolling restart.

### Limitations

- Requires Stellar Core to support a `--fd-handoff` startup flag (upstream
  contribution required). The sidecar provides the infrastructure; Core must
  consume the injected FDs.
- The handoff window introduces a brief pause in new connection acceptance
  (not in message processing).
- Not applicable to Horizon or Soroban RPC nodes (stateless HTTP servers).

## CRD Configuration

```yaml
spec:
  nodeType: Validator
  hitlessUpgrade:
    enabled: true
    handoffTimeoutSeconds: 10
    fallbackToRollingRestart: true
```

## Operator Behavior

When `hitlessUpgrade.enabled: true`:

1. The operator injects the `stellar-handoff` sidecar into the pod spec.
2. On version change, the operator sets the `stellar.org/hitless-upgrade=pending`
   annotation instead of immediately updating the StatefulSet image.
3. The sidecar orchestrates the FD handoff and signals completion via the
   `stellar.org/hitless-upgrade=complete` annotation.
4. The operator then updates the StatefulSet image and removes the annotation.

If the handoff does not complete within `handoffTimeoutSeconds`, the operator
falls back to a standard rolling restart (if `fallbackToRollingRestart: true`).

## Sidecar Image

The handoff sidecar is built from `src/sidecar.rs` and published as
`stellar-k8s/handoff-sidecar:<version>`. It is a minimal Rust binary that:

- Listens on `/handoff/sock`
- Implements the `SCM_RIGHTS` FD transfer protocol
- Exposes a `/healthz` endpoint for liveness probing

## Testing

```bash
# Run unit tests for the hitless upgrade module
cargo test -p stellar-k8s hitless_upgrade

# Integration test (requires Kind cluster)
make test-hitless-upgrade
```

## References

- [Linux `SCM_RIGHTS` man page](https://man7.org/linux/man-pages/man7/unix.7.html)
- [Envoy hot restart implementation](https://www.envoyproxy.io/docs/envoy/latest/operations/hot_restarter)
- [Stellar Core peer connection protocol](https://github.com/stellar/stellar-core/blob/master/docs/architecture.md)
