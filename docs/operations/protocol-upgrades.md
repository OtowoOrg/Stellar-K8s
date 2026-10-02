# Zero-Downtime Protocol Upgrade Runbook

This runbook covers a **network-wide Stellar protocol upgrade** for validators
and watchers managed by this operator: verifying the new `stellar-core`
release, rolling it out pod by pod, arming the upgrade at the agreed ledger
time, and recovering safely if a node falls out of sync after the version bump.

It complements, and does not replace:

- [`upgrade-guide.md`](upgrade-guide.md): general operator and node upgrades
- [`core-blue-green-runbook.md`](core-blue-green-runbook.md): Validator blue/green rollouts
- [`disaster-recovery.md`](disaster-recovery.md) and [`pvc-troubleshooting.md`](pvc-troubleshooting.md): restore paths

> Always confirm command syntax and protocol details against the official
> `stellar-core` release notes for the exact version you are deploying. Treat the
> snippets below as the operational skeleton, not a substitute for the release notes.

## How a protocol upgrade works

1. Every node must run a `stellar-core` binary that **supports** the new
   protocol version *before* the upgrade takes effect. A node on an older
   binary cannot follow the network after the bump.
2. Validators **arm** the upgrade: each one votes for the new protocol version
   at an agreed `upgradetime`. Arming is per node and is not persisted forever,
   so arm close to the agreed window.
3. When enough of the quorum has armed, the upgrade is applied at a ledger
   close after `upgradetime`. Nodes on the new binary that did not arm still
   accept an upgrade the network externalizes.
4. **Protocol versions never go backwards.** After the bump you cannot fix a
   problem by rolling the binary back. Recovery means catching up on the new
   version (see [Fail-safe procedures](#fail-safe-procedures)).

## Pre-flight checklist

- [ ] Agreed protocol version and `upgradetime` (UTC) confirmed with the other validators in your quorum
- [ ] New `stellar-core` release candidate published and its SHA-256 checksum obtained from the official release
- [ ] All `StellarNode` resources healthy: `Synced`, ledger lag within your SLO
- [ ] History archives reachable and current (needed for catch-up)
- [ ] Fresh backup or CSI VolumeSnapshot of every validator PVC (see [`backup-verification.md`](backup-verification.md))
- [ ] Rollout order decided: watchers and non-critical nodes first, validators one at a time
- [ ] `PodDisruptionBudget` in place so no more than one validator is down at once
- [ ] On-call engineer and a communication channel with peer validators available for the whole window

## Step 1: Verify the release candidate

Download the release artifact and compare its checksum against the value
published in the official release notes. Never rely on a checksum from the same
place you downloaded the binary.

```bash
# Artifact you downloaded (deb, tarball, or extracted binary)
sha256sum stellar-core-<version>.deb

# Compare against the published checksum. This must print "OK"
echo "<published-sha256>  stellar-core-<version>.deb" | sha256sum --check
```

If you build your own image, pin it by **digest**, not by a mutable tag, and
verify the binary inside the image:

```bash
docker run --rm --entrypoint stellar-core <image>@sha256:<digest> --version
```

Do not proceed if the checksum does not match.

## Step 2: Roll out the new binary (rolling, not Recreate)

Never use a `Recreate`-style rollout for validators. It takes every replica down
at once. Replace pods **sequentially** and wait for each one to be healthy and
synced before touching the next.

Change the version on one `StellarNode` at a time:

```bash
kubectl patch stellarnode <name> -n <namespace> --type merge \
  -p '{"spec":{"version":"<new-version>"}}'
```

For StatefulSet-backed workloads the update strategy must be `RollingUpdate`.
Use `partition` to control the pace if you manage the StatefulSet directly:

```yaml
spec:
  updateStrategy:
    type: RollingUpdate
    rollingUpdate:
      partition: 1   # only pods with ordinal >= partition are updated
```

Validators can alternatively use the blue/green path described in
[`core-blue-green-runbook.md`](core-blue-green-runbook.md).

After **each** pod is replaced, gate on health before continuing:

```bash
kubectl rollout status statefulset/<name> -n <namespace> --timeout=30m

kubectl exec -n <namespace> <pod> -c stellar-core -- \
  stellar-core http-command info
```

Continue only when `info` reports the node as synced and the ledger number is
advancing. Recommended order: watchers, then validators one by one.

Stop the rollout immediately if a replaced pod does not return to `Synced`.
The new binary is still safe to run on the old protocol, so investigate before
continuing.

## Step 3: Arm the upgrade

Once **all** nodes in your control run the new binary, arm the upgrade shortly
before the agreed window. Use the provided script (recommended):

```bash
examples/scripts/arm-upgrade.sh \
  --namespace <namespace> \
  --pod <validator-pod> \
  --protocol-version <N> \
  --upgrade-time 2026-10-15T14:00:00Z \
  --dry-run          # remove to apply
```

Or run the underlying `http-command` yourself:

```bash
# Arm
kubectl exec -n <namespace> <pod> -c stellar-core -- \
  stellar-core http-command \
  "upgrades?mode=set&upgradetime=2026-10-15T14:00:00Z&protocolversion=<N>"

# Verify what is armed
kubectl exec -n <namespace> <pod> -c stellar-core -- \
  stellar-core http-command "upgrades?mode=get"

# Disarm (for example if the window is postponed)
kubectl exec -n <namespace> <pod> -c stellar-core -- \
  stellar-core http-command "upgrades?mode=clear"
```

Notes:

- `upgradetime` is UTC and must be in the future.
- Arm every validator you operate, one after another, and re-run `mode=get`
  on each to confirm.
- Reach the admin HTTP port only through `kubectl exec` or `port-forward`.
  Do not expose it via a Service or Ingress.

## Step 4: Watch the bump

At and after `upgradetime`, monitor each node:

```bash
watch -n 5 "kubectl exec -n <namespace> <pod> -c stellar-core -- \
  stellar-core http-command info | jq '.info | {state, ledger: .ledger.num, version: .ledger.version}'"
```

Expected: ledgers keep closing without a gap, the reported ledger version moves
to the new protocol, and the state stays `Synced!`. A missed consensus round
shows up as the ledger number pausing well beyond the normal ~5 second close.

## Fail-safe procedures

**Because protocol versions cannot be rolled back, never downgrade the binary
after the bump.**

### A node falls out of sync right after the bump

1. Check `info`: state, ledger number, and ledger version.
2. Confirm the pod runs the **new** binary: `stellar-core --version`.
   If it is still on the old binary, roll it forward immediately.
3. Check peers and quorum: `stellar-core http-command peers` and
   `stellar-core http-command quorum`.
4. Give catch-up time to work; check logs for `catchup` progress:
   `kubectl logs <pod> -c stellar-core --tail=200`.
5. If catch-up stalls, verify history archive reachability from inside the pod.
6. If the ledger database is corrupt or stuck, restore from the most recent
   VolumeSnapshot or PVC backup **taken with the same or older protocol**,
   then let the node catch up on the new version. See
   [`pvc-troubleshooting.md`](pvc-troubleshooting.md) and
   [`disaster-recovery.md`](disaster-recovery.md).

### One validator is stuck while the rest are healthy

Do not restart the rest of the fleet. Keep the healthy validators up so the
quorum stays intact, and recover the stuck one in isolation using the steps above.

### The upgrade window must be postponed

Disarm on every node you armed (`mode=clear`), confirm with `mode=get`, and
re-coordinate a new `upgradetime` with the other validators.

### Several validators are unhealthy at once

Stop making changes. Losing quorum halts ledger progression. Escalate to
[`incident-response.md`](incident-response.md) and coordinate with peer
validators before restarting anything else.

## Post-upgrade checklist

- [ ] All nodes report the new ledger version and `Synced`
- [ ] Ledgers closing at the normal cadence, no sustained gap
- [ ] History archives publishing successfully
- [ ] No armed upgrades remain (`mode=get` returns empty on every node)
- [ ] Alerts quiet for at least one hour; document any anomalies
- [ ] Fresh post-upgrade snapshot taken

## Validating this runbook on a private network

1. Stand up a private network with at least 3 validators (for example on `kind`)
   running the **old** protocol version.
2. Run Step 1 to Step 4 against it with the new version.
3. Confirm ledger progression is uninterrupted through the bump.
4. Repeat while deliberately breaking one node (skip the binary update) and
   confirm the [fail-safe procedures](#fail-safe-procedures) recover it.
