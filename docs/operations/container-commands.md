# Container Command Configuration

## Overview

The Stellar-K8s operator now sets explicit container commands for all node types (Validator, Horizon, SorobanRpc) instead of relying on image CMD defaults. This ensures consistent behavior across different container images and provides a mechanism for custom image overrides.

## Problem Statement

Prior to this implementation:
- **Validator nodes** had explicit commands (`stellar-core run --conf`)
- **Horizon and SorobanRpc nodes** relied on the image's CMD directive
- Empty or missing image CMD caused pod crashes
- Custom images required wrapper scripts or image rebuilds

## Default Commands

The operator now sets explicit commands for each node type:

### Validator (Stellar Core)

```yaml
command:
  - /usr/bin/stellar-core
  - run
  - --conf
  - /config/stellar-core.cfg
```

### Horizon

```yaml
command:
  - /stellar-horizon
```

### Soroban RPC

```yaml
command:
  - /stellar-rpc
```

## Custom Command Override

You can override the default commands using the `spec.command` and `spec.args` fields in your StellarNode CRD.

### Basic Override Example

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: custom-validator
spec:
  nodeType: Validator
  network: Testnet
  version: v21.0.0
  
  # Override the default command
  command:
    - /custom/stellar-core
    - --config
    - /custom/config.cfg
  
  # Optional: add additional arguments
  args:
    - --verbose
    - --log-level=debug
```

### Use Cases

#### 1. Custom Image Entrypoint

If your custom image has a different binary path:

```yaml
spec:
  nodeType: Horizon
  command:
    - /opt/horizon/bin/stellar-horizon
  args:
    - --db-url
    - postgres://...
```

#### 2. Wrapper Scripts

Run your application through a wrapper script:

```yaml
spec:
  nodeType: Validator
  command:
    - /scripts/stellar-wrapper.sh
  args:
    - run
    - --conf
    - /config/stellar-core.cfg
```

#### 3. Debug Mode

Enable additional logging or debugging flags:

```yaml
spec:
  nodeType: SorobanRpc
  command:
    - /stellar-rpc
  args:
    - --log-level=debug
    - --enable-profiling
```

#### 4. Third-Party Images

Use community or vendor-specific images:

```yaml
spec:
  nodeType: Horizon
  image: registry.example.com/stellar/custom-horizon:v2.0
  command:
    - /usr/local/bin/horizon-custom
  args:
    - serve
    - --config=/etc/horizon/config.yaml
```

## Configuration Priority

The operator resolves commands in the following order:

1. **User-specified command** (`spec.command`) - highest priority
2. **User-specified args** (`spec.args`) - added to command
3. **Operator default command** - used if no override specified
4. **Image CMD** - never used (explicitly overridden)

## Validation

The operator validates command configuration:

- `command` must be a non-empty array if specified
- `args` is optional and can be empty
- First element of `command` must be an executable path

### Valid Configurations

```yaml
# Valid: Command only
command: ["/stellar-core", "run"]

# Valid: Command with args
command: ["/stellar-core"]
args: ["run", "--conf", "/config/stellar-core.cfg"]

# Valid: Full path with flags
command: ["/usr/bin/stellar-core", "run", "--conf=/config/stellar-core.cfg"]
```

### Invalid Configurations

```yaml
# Invalid: Empty command
command: []

# Invalid: Command not a string array
command: "/stellar-core run"

# Invalid: Relative path (may work but not recommended)
command: ["./stellar-core"]
```

## Testing

### Verify Command Configuration

Check the actual command used by the container:

```bash
kubectl get pod <pod-name> -o jsonpath='{.spec.containers[?(@.name=="stellar-node")].command}'
```

Check arguments:

```bash
kubectl get pod <pod-name> -o jsonpath='{.spec.containers[?(@.name=="stellar-node")].args}'
```

### Test Custom Command

1. Apply a StellarNode with custom command:
   ```bash
   kubectl apply -f custom-command-node.yaml
   ```

2. Verify the pod starts successfully:
   ```bash
   kubectl get pods -l app.kubernetes.io/instance=custom-validator
   ```

3. Check container logs:
   ```bash
   kubectl logs <pod-name> -c stellar-node
   ```

4. Verify the process is running with correct args:
   ```bash
   kubectl exec <pod-name> -- ps aux | grep stellar
   ```

## Unit Tests

The implementation includes comprehensive unit tests:

```rust
#[test]
fn test_validator_has_explicit_command() { ... }

#[test]
fn test_horizon_has_explicit_command() { ... }

#[test]
fn test_soroban_has_explicit_command() { ... }

#[test]
fn test_custom_command_override() { ... }
```

Run tests with:
```bash
cargo test -p stellar-k8s container_command
```

## Migration Guide

### Existing Nodes

Existing StellarNode resources will automatically receive explicit commands during the next reconciliation. No manual intervention required.

### Rolling Update Behavior

When the operator adds explicit commands:
1. StatefulSet/Deployment spec is updated
2. Kubernetes triggers a rolling update
3. New pods start with explicit commands
4. Old pods are terminated gracefully

### Rollback

If issues occur, you can temporarily override the command:

```bash
kubectl patch stellarnode <name> --type=merge -p '{
  "spec": {
    "command": ["/usr/bin/stellar-core", "run", "--conf", "/config/stellar-core.cfg"]
  }
}'
```

## Troubleshooting

### Pod Crashes on Startup

**Symptom**: Pod enters CrashLoopBackOff after applying custom command

**Diagnosis**:
```bash
kubectl logs <pod-name> -c stellar-node --previous
```

**Common Causes**:
1. Invalid executable path
2. Missing required arguments
3. Configuration file not found
4. Permission issues

**Resolution**:
```bash
# Check if the binary exists
kubectl exec <pod-name> -- ls -la /path/to/binary

# Check file permissions
kubectl exec <pod-name> -- ls -la /config/

# Test command manually
kubectl exec <pod-name> -- /path/to/binary --help
```

### Command Not Applied

**Symptom**: Pod still uses old command after update

**Diagnosis**:
```bash
kubectl get stellarnode <name> -o yaml | grep -A 5 "command:"
kubectl get pod <pod-name> -o yaml | grep -A 5 "command:"
```

**Common Causes**:
1. Resource not reconciled yet
2. Pod not restarted
3. Validation error preventing update

**Resolution**:
```bash
# Force reconciliation
kubectl annotate stellarnode <name> stellar.org/force-reconcile="$(date +%s)"

# Delete pod to force recreation
kubectl delete pod <pod-name>
```

### Args Not Passed Correctly

**Symptom**: Application doesn't receive expected arguments

**Diagnosis**:
```bash
# Check actual command line
kubectl exec <pod-name> -- ps -ef | grep stellar

# Check container spec
kubectl get pod <pod-name> -o jsonpath='{.spec.containers[0].args}'
```

**Resolution**:
Ensure args are properly formatted as a string array:
```yaml
args:
  - "--flag1"
  - "--flag2=value"
  - "positional-arg"
```

## Best Practices

### 1. Use Absolute Paths

Always use absolute paths for commands:
```yaml
command: ["/usr/bin/stellar-core"]  # Good
command: ["stellar-core"]            # Avoid
```

### 2. Separate Command and Args

For clarity, separate the executable from its arguments:
```yaml
command: ["/stellar-core"]
args: ["run", "--conf", "/config/stellar-core.cfg"]
```

### 3. Test in Non-Production First

Always test custom commands in a development environment:
```yaml
metadata:
  name: test-custom-command
spec:
  network: Testnet  # Test on testnet first
  command: ["/custom/stellar-core"]
```

### 4. Document Custom Commands

Add annotations explaining why custom commands are needed:
```yaml
metadata:
  annotations:
    stellar.org/custom-command-reason: "Using vendor-specific image with different entrypoint"
```

### 5. Version Control

Keep custom command configurations in version control:
```yaml
# stellarnode-custom.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: mainnet-validator-01
spec:
  command: ["/usr/bin/stellar-core"]
  args: ["run", "--conf", "/config/stellar-core.cfg"]
```

## Security Considerations

### Command Injection

The operator does not perform shell expansion, preventing command injection:
```yaml
# Safe: Each element is treated as literal string
command: ["/bin/sh", "-c", "echo $HOME"]

# Not executed through shell
args: ["run && malicious-command"]  # Treated as single argument
```

### File Path Validation

Ensure commands reference trusted binaries:
- Use absolute paths
- Verify binary exists in image
- Avoid writable directories

### Resource Limits

Custom commands should respect container resource limits:
```yaml
spec:
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
  command: ["/custom/stellar-core"]
```

## Examples

### Complete Examples Repository

See `examples/custom-commands/` for complete working examples:

- `validator-custom-entrypoint.yaml` - Custom validator binary path
- `horizon-wrapper-script.yaml` - Horizon with initialization wrapper
- `soroban-debug-mode.yaml` - SorobanRpc with debug flags
- `third-party-image.yaml` - Community image integration

## Related Documentation

- [Image Pinning](../image-pinning.md)
- [Resource Limits](../resource-limits.md)
- [Sidecars](../sidecars.md)

## References

- [Kubernetes Container Commands](https://kubernetes.io/docs/tasks/inject-data-application/define-command-argument-container/)
- [Docker CMD vs ENTRYPOINT](https://docs.docker.com/engine/reference/builder/#understand-how-cmd-and-entrypoint-interact)
- [Issue #1558: Container Command for All Node Types](https://github.com/OtowoOrg/Stellar-K8s/issues/1558)
