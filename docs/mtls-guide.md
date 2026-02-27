# mTLS Setup Guide for the Stellar-K8s Operator REST API

This guide walks through enabling, verifying, and troubleshooting **mutual TLS (mTLS)** on the operator's built-in REST API. When mTLS is active the operator:

1. Generates a self-signed **Certificate Authority (CA)**.
2. Issues a **server certificate** signed by that CA.
3. Stores both as Kubernetes `Secret` resources.
4. Starts the REST API on port **8443** with TLS termination and optional client-certificate verification.

---

## Prerequisites

| Tool | Version |
|------|---------|
| Rust | 1.88+ |
| kubectl | configured against a running cluster |
| curl | any recent version with TLS support |
| A Kubernetes cluster | 1.28+ (KinD, minikube, or remote) |

The operator must have RBAC permissions to **create** and **get** `Secrets` in the target namespace. The default Helm chart and the E2E manifests already include these permissions.

---

## How It Works

```
┌──────────────────────┐        ┌──────────────────────────┐
│   Operator Startup   │        │    Kubernetes Secrets     │
│                      │        │                           │
│  --enable-mtls flag  │───────▶│  stellar-operator-ca      │
│                      │        │    tls.crt  (CA cert)     │
│  1. ensure_ca()      │        │    tls.key  (CA key)      │
│  2. ensure_server_   │        │                           │
│     cert()           │───────▶│  stellar-operator-server- │
│  3. Load PEM into    │        │  cert                     │
│     RustlsConfig     │        │    tls.crt  (server cert) │
│  4. Bind :8443       │        │    tls.key  (server key)  │
└──────────────────────┘        │    ca.crt   (CA cert)     │
                                └──────────────────────────┘
```

### Certificate details

- **CA** — Common Name `stellar-operator-ca`, key usages: `DigitalSignature`, `KeyCertSign`, `CrlSign`.
- **Server cert** — Common Name `stellar-operator`, signed by the CA. SANs include `localhost`, the operator Service name, and the full cluster-local DNS names. Extended key usages: `ServerAuth`, `ClientAuth`.
- **Node client certs** (issued per `StellarNode`) — Common Name `stellar-node-<name>`, signed by the same CA, with `ClientAuth` and `ServerAuth` extended key usages.

Client certificate verification is **optional** (`allow_unauthenticated`). Clients that present a valid cert signed by the CA are verified; clients without a cert are still allowed to connect over TLS.

---

## Running the Operator with mTLS

### Option A — Local binary (development)

```bash
# Build the operator
cargo build --release --bin stellar-operator

# Run with mTLS enabled, targeting the 'default' namespace
./target/release/stellar-operator run \
  --enable-mtls \
  --namespace default
```

### Option B — Helm (production)

Set the `mtls.enabled` value when installing the chart:

```bash
helm install stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set mtls.enabled=true
```

Or use the `ENABLE_MTLS` environment variable in the Deployment manifest:

```yaml
env:
  - name: ENABLE_MTLS
    value: "true"
  - name: OPERATOR_NAMESPACE
    value: "stellar-system"
```

---

## Verifying the Setup

### Step 1 — Confirm the Secrets exist

After the operator starts with `--enable-mtls`, two Secrets are created in the operator namespace:

```bash
kubectl get secrets -n default
```

Expected output includes:

```
NAME                              TYPE     DATA   AGE
stellar-operator-ca               Opaque   2      10s
stellar-operator-server-cert      Opaque   3      10s
```

The CA secret contains `tls.crt` and `tls.key`. The server-cert secret also includes `ca.crt` for convenience.

### Step 2 — Extract the CA certificate

```bash
kubectl get secret stellar-operator-ca -n default \
  -o jsonpath='{.data.tls\.crt}' | base64 -d > ca.crt
```

### Step 3 — Test with curl (server TLS verification)

```bash
curl --cacert ca.crt https://localhost:8443/healthz
```

Expected response:

```
ok
```

You can also hit the full health endpoint:

```bash
curl --cacert ca.crt https://localhost:8443/health
```

Expected response:

```json
{"status":"healthy","version":"0.1.0"}
```

### Step 4 — Test with full mTLS (client certificate)

To exercise the mutual part of mTLS, extract a node client cert (or generate one manually) and pass it to curl:

```bash
# Extract client cert and key (example for a node called "my-validator")
kubectl get secret my-validator-client-cert -n default \
  -o jsonpath='{.data.tls\.crt}' | base64 -d > client.crt
kubectl get secret my-validator-client-cert -n default \
  -o jsonpath='{.data.tls\.key}' | base64 -d > client.key

# Full mTLS call
curl --cacert ca.crt \
     --cert client.crt \
     --key client.key \
     https://localhost:8443/health
```

---

## Available Endpoints

When the `rest-api` feature is enabled (it is by default), the following endpoints are served:

| Path | Method | Description |
|------|--------|-------------|
| `/healthz` | GET | Lightweight liveness probe — returns `ok` |
| `/health` | GET | JSON health check with version info |
| `/leader` | GET | Leader election status |
| `/metrics` | GET | Prometheus metrics (text format) |
| `/api/v1/nodes` | GET | List all `StellarNode` resources |
| `/api/v1/nodes/:namespace/:name` | GET | Get a specific `StellarNode` |

---

## Kubernetes Secrets Reference

### `stellar-operator-ca`

| Key | Description |
|-----|-------------|
| `tls.crt` | PEM-encoded CA certificate |
| `tls.key` | PEM-encoded CA private key |

### `stellar-operator-server-cert`

| Key | Description |
|-----|-------------|
| `tls.crt` | PEM-encoded server certificate |
| `tls.key` | PEM-encoded server private key |
| `ca.crt` | PEM-encoded CA certificate (copy) |

### `<node-name>-client-cert` (per StellarNode)

| Key | Description |
|-----|-------------|
| `tls.crt` | PEM-encoded client certificate |
| `tls.key` | PEM-encoded client private key |
| `ca.crt` | PEM-encoded CA certificate (copy) |

---

## Troubleshooting

### Certificate hostname mismatch

If curl reports a hostname verification error, the server certificate SAN list may not cover the hostname you are connecting to. The operator includes the following SANs by default:

- `localhost`
- `stellar-operator`
- `stellar-operator.<namespace>`
- `stellar-operator.<namespace>.svc`
- `stellar-operator.<namespace>.svc.cluster.local`

For local development, `localhost` is the correct hostname. Inside the cluster, use the Service DNS name.

### Secrets not created

- Ensure the operator ServiceAccount has `create`, `get`, and `patch` permissions on `secrets` in the target namespace.
- Check operator logs: `kubectl logs deploy/stellar-operator -n <namespace>`.

### Connection refused on port 8443

- Confirm you passed `--enable-mtls` (or set `ENABLE_MTLS=true`). Without it the server starts on port **8080** in plain HTTP mode.

### Client certificate rejected

- Verify the client cert was signed by the same CA that the operator trusts.
- Check that the cert has not expired (`openssl x509 -in client.crt -noout -dates`).

---

## Security Considerations

- The CA private key is stored as a Kubernetes Secret. Restrict access via RBAC.
- Certificates are generated at operator startup and reused on subsequent runs (idempotent).
- Node client-cert Secrets have `ownerReferences` pointing to the `StellarNode`, so they are garbage-collected when the node is deleted.
- Client certificate verification is optional — the server accepts both authenticated and unauthenticated TLS connections. To enforce strict mTLS, remove the `allow_unauthenticated()` call in `src/rest_api/server.rs`.

---

## Source Code Reference

- [`src/controller/mtls.rs`](../src/controller/mtls.rs) — CA and certificate generation logic.
- [`src/main.rs`](../src/main.rs) — mTLS bootstrapping during operator startup.
- [`src/rest_api/server.rs`](../src/rest_api/server.rs) — TLS server configuration with `rustls`.
