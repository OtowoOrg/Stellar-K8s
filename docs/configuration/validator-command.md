# Validator Container Command Override

The operator injects an explicit command into every validator `Pod` spec.
This page explains why that is necessary, how the default command is
constructed, where the config file comes from, and when — and how — to
change it.

---

## Why the operator sets an explicit command

The official `stellar/stellar-core` container image ships with an **empty
`Cmd`** (and an empty `Entrypoint`). If Kubernetes were left to use the
image defaults, the container would start with no executable and exit
immediately with code `0` — an immediate-exit loop that surfaces as a
`CrashLoopBackOff`.

To prevent silent no-ops, the operator always injects a concrete command
into the generated `Pod` spec for every validator workload. You must never
rely on the image default for validator containers.

!!! warning "Do not rely on the image default `Cmd`"
    The `stellar/stellar-core` image intentionally ships without a default
    command. If you build or pull a custom image and remove the operator's
    command injection, the container will exit immediately.
    See [Container exits immediately](#container-exits-immediately) in the
    troubleshooting section below.

---

## Default command

For `NodeType::Validator` workloads the operator generates this command:

```
/usr/bin/stellar-core run --conf /config/stellar-core.cfg
```

Expressed as a Kubernetes `command` array in the rendered `Pod` spec:

```yaml
containers:
  - name: stellar-node
    image: stellar/stellar-core:<tag>
    command:
      - /usr/bin/stellar-core
      - run
      - --conf
      - /config/stellar-core.cfg
```

| Field | Value | Notes |
|-------|-------|-------|
| Binary | `/usr/bin/stellar-core` | Standard install path inside the official image |
| Subcommand | `run` | Starts the node in continuous operation mode |
| Flag | `--conf` | Points `stellar-core` at its configuration file |
| Config path | `/config/stellar-core.cfg` | Fixed mount point — see [Config file path](#config-file-path) below |

---

## Config file path

The config file is always mounted at `/config/stellar-core.cfg`.

The operator renders the node's configuration as a Kubernetes `ConfigMap`
and mounts it into the container at `/config/`. The key inside the
`ConfigMap` is `stellar-core.cfg`, which maps to the file path
`/config/stellar-core.cfg` at runtime:

```
ConfigMap key:  stellar-core.cfg
Mount point:    /config/
Runtime path:   /config/stellar-core.cfg   ← what --conf points at
```

!!! note "The path is not configurable via `spec.config`"
    The `/config/stellar-core.cfg` path is fixed by the operator's
    `ConfigMap` volume mount. If your custom image expects a different path
    you must use a [command override](#overriding-the-command) to match it.

---

## Overriding the command

The `spec.command` and `spec.args` fields on `StellarNode` let you replace
the default command entirely when the standard flags are insufficient.
Override **only** when you have a specific requirement the defaults cannot
meet — for example:

- A custom image that installs `stellar-core` at a non-standard path.
- A wrapper script that performs pre-flight checks before starting the node.
- Non-standard flag combinations required by an experimental build.

### Override via `spec.command`

```yaml
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarNode
metadata:
  name: validator-custom
  namespace: stellar
spec:
  nodeType: Validator
  command:
    - /opt/stellar/bin/stellar-core
    - run
    - --conf
    - /config/stellar-core.cfg
```

### Override with a wrapper script

```yaml
spec:
  nodeType: Validator
  command:
    - /bin/sh
    - -c
  args:
    - |
      echo "Starting stellar-core"
      exec /usr/bin/stellar-core run --conf /config/stellar-core.cfg
```

### Using `spec.args` alone

`spec.args` replaces the arguments passed to the image `Entrypoint`. It is
most useful when the image already sets a correct entrypoint binary but you
need to pass extra flags:

```yaml
spec:
  nodeType: Validator
  args:
    - run
    - --conf
    - /config/stellar-core.cfg
    - --verbose
```

!!! tip "Override in the workload spec, not the image"
    Command customization belongs in the `StellarNode` spec. Avoid baking
    custom entrypoints into the container image — that makes the image
    harder to reuse across environments and breaks the operator's ability
    to inject security-hardened defaults.

---

## Precedence

When the operator builds the container spec it applies the following
precedence:

```
spec.command (user-supplied)  →  used as-is; default is discarded
spec.command (absent)         →  operator default for the node type
```

The same rule applies to `spec.args`. The operator does **not** merge
user-supplied values with the defaults — a non-empty `spec.command`
replaces the entire default command array.

---

## Troubleshooting

### Container exits immediately

**Symptom**

```
validator-0   0/1   CrashLoopBackOff
```

Logs show no output, or output ends with `exit code 0` or `exit code 1`
immediately after start.

**Root causes and fixes**

| Root cause | How to diagnose | Fix |
|------------|-----------------|-----|
| Image has no default `Cmd` and operator command was accidentally cleared | `kubectl get pod validator-0 -o jsonpath='{.spec.containers[0].command}'` returns empty | Restore the default command or set `spec.command` explicitly |
| Wrong binary path in `spec.command` | `kubectl exec validator-0 -- ls /usr/bin/stellar-core` | Correct the path in `spec.command` |
| Config file not found at `/config/stellar-core.cfg` | `kubectl exec validator-0 -- ls /config/` | Verify the `ConfigMap` volume is mounted; check `kubectl describe pod validator-0` for mount errors |
| Custom wrapper script exits before exec | `kubectl logs validator-0 --previous` | Fix the wrapper script; always use `exec` before the final command so the process takes PID 1 |

**Verify the rendered command**

```bash
kubectl get pod validator-0 -n stellar \
  -o jsonpath='{.spec.containers[0].command}' | python3 -m json.tool
```

Expected output for a standard deployment:

```json
[
  "/usr/bin/stellar-core",
  "run",
  "--conf",
  "/config/stellar-core.cfg"
]
```

**Verify the ConfigMap is mounted**

```bash
# List the ConfigMap that backs the config volume
kubectl get configmap -n stellar -l stellar.org/node-name=validator-0

# Check the file is present inside the container
kubectl exec -n stellar validator-0 -- cat /config/stellar-core.cfg | head -5
```

See also: [Common Issues — Issue 3: ImagePullBackOff](../troubleshooting/common-issues.md) and
the full [Troubleshooting Guide](../troubleshooting/common-issues.md) for
broader deployment problems.
