# kubectl-stellar Interactive Mode Guide

The kubectl-stellar plugin now includes an interactive mode that provides a menu-driven interface with guided workflows for common operations.

## Overview

Interactive mode simplifies complex operations by:
- Providing step-by-step guidance
- Validating inputs in real-time
- Offering contextual help and suggestions
- Reducing the need to memorize command syntax
- Including progress indicators for long operations
- Adding confirmation prompts for destructive actions

## Launching Interactive Mode

```bash
# Start interactive mode
kubectl stellar interactive

# Or use the shorthand
kubectl stellar -i
```

## Features

### 1. Deploy New Stellar Node

Guided workflow for deploying validators, Horizon, or Soroban RPC nodes:

**Steps:**
1. Select node type (Validator/Horizon/Soroban RPC)
2. Enter node name
3. Choose namespace
4. Select Stellar network (mainnet/testnet/futurenet)
5. Configure storage size
6. Review generated manifest
7. Confirm deployment

**Example:**
```
🚀 Deploy New Stellar Node
This wizard will guide you through deploying a new node.

? Select node type › Validator
? Enter node name › my-validator
? Enter namespace › stellar-mainnet
? Select Stellar network › mainnet
? Enter storage size (e.g., 100Gi) › 500Gi

📝 Generating manifest...
[manifest preview]

? Deploy this node? › Yes

✓ Node deployed successfully!
```

### 2. View Node Status

Multiple viewing options with smart filtering:

**Options:**
- All nodes in current namespace
- All nodes across all namespaces
- Specific node by name
- Nodes filtered by type (validator/horizon/soroban)

**Features:**
- Color-coded status indicators
- Real-time updates
- Sortable columns
- Export to JSON/YAML

### 3. Troubleshooting Wizard

Interactive diagnostics for problem resolution:

**Diagnostic Checks:**
1. Node existence verification
2. Pod status analysis
3. Recent events review
4. PVC status check
5. Resource utilization
6. Network connectivity

**Available Actions:**
- View detailed logs
- Describe node resource
- Check operator logs
- Restart node pods
- Delete and recreate node

**Example:**
```
🔧 Troubleshooting Wizard
? Enter node name to troubleshoot › my-validator

Running diagnostics...

1. Checking if node exists...
✓ Node exists

2. Checking pod status...
NAME                          READY   STATUS    RESTARTS   AGE
my-validator-0                1/1     Running   0          5m

3. Checking recent events...
[events list]

4. Checking PVC status...
[PVC status]

? Select an action ›
❯ View detailed logs
  Describe node resource
  Check operator logs
  Restart node pods
  Delete and recreate node
  Back to main menu
```

### 4. Scale Horizon Deployment

Simple scaling interface with validation:

**Features:**
- Current replica count display
- Recommended replica ranges
- Confirmation before scaling
- Progress tracking
- Rollback option

**Example:**
```
📈 Scale Horizon Deployment
? Enter Horizon node name › horizon-api
? Enter desired replica count › 5
? Scale horizon-api to 5 replicas? › Yes

Scaling deployment...
✓ Deployment scaled successfully!
```

### 5. Backup and Restore

Guided backup and restore operations:

**Operations:**
- Create VolumeSnapshot backup
- List available backups
- Restore from backup
- Schedule automated backups

**Example:**
```
💾 Backup and Restore
? Select operation ›
❯ Create backup
  List backups
  Restore from backup

? Enter node name to backup › my-validator

Creating VolumeSnapshot...
✓ Snapshot created: my-validator-snapshot-20260530
```

### 6. View Logs

Enhanced log viewing with filtering:

**Options:**
- Tail specific number of lines
- Follow logs in real-time
- Filter by log level
- Search within logs
- Export logs to file

**Example:**
```
📜 View Logs
? Enter node name › my-validator
? Follow logs (stream)? › No
? Number of lines to show › 100

Fetching logs...
[log output]
```

### 7. Network Diagnostics

Network health and topology visualization:

**Diagnostics:**
- View network topology (ASCII/Graphviz)
- Check peer connections
- Test connectivity
- View SCP metrics
- Analyze consensus health

**Example:**
```
🌐 Network Diagnostics
? Select diagnostic ›
❯ View network topology
  Check peer connections
  Test connectivity
  View SCP metrics

Generating network topology...
[ASCII topology diagram]
```

## Tab Completion

Interactive mode supports tab completion for:
- Node names
- Namespaces
- Network names
- Storage classes
- Common values

Enable tab completion:

```bash
# Bash
kubectl stellar completions bash > /etc/bash_completion.d/kubectl-stellar

# Zsh
kubectl stellar completions zsh > ~/.zsh/completions/_kubectl-stellar

# Fish
kubectl stellar completions fish > ~/.config/fish/completions/kubectl-stellar.fish
```

## Colored Output

Interactive mode uses colors for better readability:

- 🟢 **Green**: Success messages, healthy status
- 🔴 **Red**: Errors, critical issues
- 🟡 **Yellow**: Warnings, in-progress operations
- 🔵 **Cyan**: Informational messages, headers
- ⚫ **Gray**: Secondary information, hints

Disable colors:
```bash
export NO_COLOR=1
kubectl stellar interactive
```

## Progress Indicators

Long-running operations show progress:

```
🚀 Deploying node...
[████████████████████░░░░] 80% - Creating PVC
```

## Confirmation Prompts

Destructive actions require confirmation:

```
⚠ WARNING: This will delete and recreate the node.
  Data will be preserved if using persistent volumes.

? Are you sure? › No
```

## Keyboard Shortcuts

- **↑/↓**: Navigate menu options
- **Enter**: Select option
- **Esc**: Cancel/Go back
- **Ctrl+C**: Exit interactive mode
- **Tab**: Auto-complete (where supported)
- **?**: Show help for current screen

## Configuration

Customize interactive mode behavior:

```bash
# Set default namespace
export KUBECTL_STELLAR_NAMESPACE=stellar-mainnet

# Set default output format
export KUBECTL_STELLAR_OUTPUT=table

# Enable verbose mode
export KUBECTL_STELLAR_VERBOSE=true

# Set custom theme
export KUBECTL_STELLAR_THEME=dark
```

## Examples

### Complete Deployment Workflow

```bash
$ kubectl stellar interactive

╔═══════════════════════════════════════════════════════════╗
║  ✦ Stellar-K8s Interactive Mode                          ║
║  Cloud-Native Stellar Infrastructure on Kubernetes       ║
╚═══════════════════════════════════════════════════════════╝

? What would you like to do? ›
❯ Deploy a new Stellar node
  View node status
  Troubleshoot a node
  Scale Horizon deployment
  Backup and restore
  View logs
  Network diagnostics
  Exit

[Select "Deploy a new Stellar node"]

🚀 Deploy New Stellar Node
? Select node type › Validator
? Enter node name › mainnet-validator-1
? Enter namespace › stellar-mainnet
? Select Stellar network › mainnet
? Enter storage size › 1Ti

📝 Generating manifest...
[manifest shown]

? Deploy this node? › Yes

✓ Node deployed successfully!

Next steps:
  • Check status: kubectl stellar status mainnet-validator-1
  • View logs: kubectl stellar logs mainnet-validator-1
  • Monitor: kubectl stellar status --watch
```

### Troubleshooting Workflow

```bash
$ kubectl stellar interactive

? What would you like to do? › Troubleshoot a node

🔧 Troubleshooting Wizard
? Enter node name to troubleshoot › failing-node

Running diagnostics...

1. Checking if node exists...
✓ Node exists

2. Checking pod status...
NAME                    READY   STATUS             RESTARTS   AGE
failing-node-0          0/1     CrashLoopBackOff   5          10m

3. Checking recent events...
Warning  BackOff  Pod  Back-off restarting failed container

4. Checking PVC status...
✓ PVC bound successfully

? Select an action › View detailed logs

[Shows last 100 lines of logs with error highlighted]

ERROR: Failed to connect to database
```

## Best Practices

1. **Use Interactive Mode for Learning**: Great for understanding available options
2. **Validate Before Applying**: Always review generated manifests
3. **Start with Dry Runs**: Test operations in non-production first
4. **Keep Sessions Short**: Exit and re-enter for fresh context
5. **Use Tab Completion**: Speeds up input and reduces errors
6. **Read Confirmation Prompts**: Understand what will happen before confirming

## Troubleshooting Interactive Mode

### Interactive Mode Won't Start

**Problem**: Command hangs or fails to start

**Solutions:**
```bash
# Check if terminal supports interactive mode
echo $TERM

# Try with explicit terminal type
TERM=xterm-256color kubectl stellar interactive

# Check for conflicting environment variables
unset KUBECTL_STELLAR_*
```

### Colors Not Displaying

**Problem**: Output shows escape codes instead of colors

**Solutions:**
```bash
# Enable color support
export TERM=xterm-256color

# Or disable colors
export NO_COLOR=1
```

### Tab Completion Not Working

**Problem**: Tab key doesn't auto-complete

**Solutions:**
```bash
# Reinstall completions
kubectl stellar install-completion bash

# Reload shell
source ~/.bashrc
```

## Related Documentation

- [kubectl-stellar Plugin Guide](../kubectl-plugin.md)
- Deployment Guide
- Troubleshooting Guide

## Feedback

Interactive mode is continuously improving. Submit feedback:
- GitHub Issues: https://github.com/OtowoOrg/Stellar-K8s/issues
- Feature Requests: Use the "enhancement" label
