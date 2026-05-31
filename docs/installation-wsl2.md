# Windows (WSL2) Setup Guide

This guide covers installing and running Stellar-K8s on Windows using WSL2. Linux and macOS users should follow the standard [Getting Started](getting-started.md) guide instead.

---

## Prerequisites

| Requirement | Minimum version |
|---|---|
| Windows | 10 (build 19041) or Windows 11 |
| WSL | 2 (not WSL 1) |
| Docker Desktop | 4.x with WSL2 backend |
| Rust | 1.88+ (installed inside WSL2) |
| kubectl | Latest stable (installed inside WSL2) |

---

## 1. Enable WSL2

Open **PowerShell as Administrator** and run:

```powershell
wsl --install
wsl --set-default-version 2
```

Then install a distribution (Ubuntu 22.04 recommended):

```powershell
wsl --install -d Ubuntu-22.04
```

> **Troubleshooting — Virtual Machine Platform not enabled**
>
> If `wsl --install` fails with a virtualisation error:
>
> 1. Open **Windows Features** (`optionalfeatures.exe`) and enable:
>    - **Virtual Machine Platform**
>    - **Windows Subsystem for Linux**
> 2. Reboot.
> 3. If the error persists, enter your BIOS/UEFI and enable **Intel VT-x** or **AMD-V** (sometimes labelled *SVM Mode*).
> 4. Re-run `wsl --install` after rebooting.

Verify WSL2 is active:

```powershell
wsl --list --verbose
# NAME            STATE   VERSION
# Ubuntu-22.04    Running 2        <-- must be 2
```

---

## 2. Enable Docker Desktop WSL2 Backend

1. Install [Docker Desktop for Windows](https://www.docker.com/products/docker-desktop/).
2. Open Docker Desktop → **Settings** → **General** → enable **Use the WSL 2 based engine**.
3. Go to **Settings** → **Resources** → **WSL Integration** → toggle on your Ubuntu distribution.
4. Click **Apply & Restart**.

Verify inside WSL2:

```bash
docker info | grep -i "server version"
# Server Version: 24.x.x
```

---

## 3. Configure a Local Kubernetes Cluster

### Option A — Docker Desktop Kubernetes (simplest)

Enable it in Docker Desktop → **Settings** → **Kubernetes** → **Enable Kubernetes**.

The kubeconfig is automatically written to `~/.kube/config` inside WSL2.

### Option B — Minikube with the `docker` driver

```bash
# Install minikube inside WSL2
curl -LO https://storage.googleapis.com/minikube/releases/latest/minikube-linux-amd64
sudo install minikube-linux-amd64 /usr/local/bin/minikube

# Start with the docker driver (recommended for WSL2)
minikube start --driver=docker --cpus=4 --memory=8192

# Verify
kubectl get nodes
```

> **Why `--driver=docker` and not `--driver=none`?**
> The `none` driver requires running as root and bypasses container isolation. Use it only in CI environments. For local development, `docker` is safer and fully supported inside WSL2.

---

## 4. WSL2 Networking

WSL2 uses a **NAT-based virtual network**. The WSL2 VM gets a private IP (e.g. `172.x.x.x`) that changes on every Windows restart. This affects how you reach the operator's metrics endpoint and admission webhook from the Windows host.

### Accessing services from Windows

```bash
# Inside WSL2 — find the current WSL IP
ip addr show eth0 | grep "inet " | awk '{print $2}' | cut -d/ -f1
# e.g. 172.28.144.10
```

Use that IP from Windows browsers or tools (e.g. `http://172.28.144.10:8080`).

### Using `localhost` from WSL2

Services bound to `0.0.0.0` inside WSL2 are reachable at `localhost` **from within WSL2 itself**. From Windows, use the WSL IP above or configure port forwarding:

```powershell
# PowerShell (Admin) — forward Windows port 9090 to WSL2 port 9090
netsh interface portproxy add v4tov4 listenport=9090 listenaddress=0.0.0.0 connectport=9090 connectaddress=<WSL_IP>
```

### Exposing the operator metrics and webhook

When running the operator locally (`make run-local`), bind to `0.0.0.0` so the metrics endpoint is reachable from the cluster:

```bash
stellar-operator run --namespace stellar-system
# Metrics default: http://0.0.0.0:8080/metrics

stellar-operator webhook --bind 0.0.0.0:8443 \
  --cert-path /tls/tls.crt --key-path /tls/tls.key
```

For the admission webhook to be reachable by the Kubernetes API server running inside Docker Desktop or Minikube, use the WSL IP (not `localhost`) in the `ValidatingWebhookConfiguration.webhooks[].clientConfig.url` field:

```yaml
clientConfig:
  url: "https://172.28.144.10:8443/validate"
```

---

## 5. Install Rust and Build the Operator

All commands below run **inside WSL2**:

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Verify
rustc --version   # must be 1.88+

# Clone and build
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
make build
```

---

## 6. Troubleshooting

### File system performance — avoid `/mnt/c/`

WSL2 accesses Windows drives (e.g. `C:\`) via a 9P network protocol. Running `cargo build` from `/mnt/c/Users/you/projects/` is **significantly slower** (often 10–20×) than running from the Linux filesystem.

**Always clone and work from your Linux home directory:**

```bash
# Good
cd ~/projects && git clone https://github.com/OtowoOrg/Stellar-K8s.git

# Slow — avoid
cd /mnt/c/Users/you/projects && git clone ...
```

If you have an existing clone under `/mnt/c/`, copy it:

```bash
cp -r /mnt/c/Users/you/projects/Stellar-K8s ~/projects/Stellar-K8s
```

### Clock desync breaking Kubernetes certificates

WSL2 inherits the Windows system clock, but the VM clock can drift when the host sleeps or hibernates. A skewed clock causes TLS certificate validation failures in Kubernetes (e.g. `certificate has expired or is not yet valid`).

**Fix — resync the clock manually:**

```bash
sudo hwclock --hctosys
# or
sudo ntpdate pool.ntp.org
```

**Fix — resync automatically on WSL2 startup** by adding to `~/.bashrc` or `~/.zshrc`:

```bash
# Resync clock on shell start (WSL2 clock drift workaround)
sudo hwclock --hctosys 2>/dev/null || true
```

To allow this without a password prompt, add to `/etc/sudoers` (run `sudo visudo`):

```
%sudo ALL=(ALL) NOPASSWD: /sbin/hwclock
```

### `minikube start` fails with "Exiting due to PROVIDER_DOCKER_NOT_RUNNING"

Docker Desktop is not running or WSL integration is not enabled for your distro. Check:

1. Docker Desktop is open and the whale icon in the system tray shows **Running**.
2. **Settings → Resources → WSL Integration** has your distro toggled on.
3. Run `docker ps` inside WSL2 — if it hangs or errors, restart Docker Desktop.

### kubectl cannot connect after Windows restart

The WSL IP changes on restart. If you hard-coded the WSL IP anywhere (webhook URL, port-proxy rules), update it:

```bash
# Get new IP
ip addr show eth0 | grep "inet " | awk '{print $2}' | cut -d/ -f1
```

For Docker Desktop Kubernetes, the kubeconfig is refreshed automatically — just run `kubectl get nodes` to confirm connectivity.

---

## Next Steps

- [Quick Start](../README.md#-quick-start) — deploy your first `StellarNode`
- [kubectl-stellar plugin](kubectl-plugin.md) — manage nodes from the CLI
- [Monitoring & Observability](../README.md#-monitoring--observability) — import the Grafana dashboard
