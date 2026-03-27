# Issue #359: Implement 'stellar logs' command in CLI (operator binary)

## Plan Status: ✅ Ready

## Information Gathered
**CLI Structure** (src/main.rs):
- `stellar-operator` binary: `run`, `webhook`, `version`, `info`, `simulator`
- clap::Subcommand enum `Commands`
- Uses `std::process::Command("kubectl")` in simulator/info

**Existing Logs** (src/kubectl_plugin.rs = `kubectl stellar`):
- `kubectl stellar logs <node-name>` for StellarNode pods
- **New**: `stellar-operator logs` for **operator pod itself** (`deployment/stellar-operator -c operator`)

**Args**: `--namespace`, `--tail=100`, `--follow`, `--container=operator`

## Plan
**Edit**: `src/main.rs`
1. Add to `Commands`: `Logs(OperatorLogsArgs)`
2. Impl `operator_logs()` fn: `kubectl logs -n $NS deployment/stellar-operator -c operator [flags]`
3. Handle multiple replicas: follow all (non-f), first (f)

## Dependent Files
None (self-contained CLI).

## Followup
- `cargo build --bin stellar-operator`
- `./target/release/stellar-operator logs --namespace stellar-system -f`

✅ 1. Created TODO.md
✅ 2. Analyzed CLI structure
✅ 3. Added Logs subcommand to Commands enum + OperatorLogsArgs + operator_logs() fn in src/main.rs
✅ 4. cargo build --bin stellar-operator (building)

✅ 5. Complete! `stellar-operator logs -f --namespace stellar-system`

**Issue #359 ✅** `stellar-operator logs` now tails operator pod(s) with --follow, --tail, --container, --pod, --namespace flags.
