# API Versioning Strategy

*Addresses issue #1419.*

This document defines the versioning scheme, deprecation timeline, and
migration path for the Stellar-K8s operator REST API.

---

## Versioning Scheme

The operator REST API uses **URL-path versioning** as the primary strategy.

```
https://<operator-host>:<port>/api/{version}/{resource}
```

Version identifiers are monotonically increasing integers prefixed with `v`:
`v1`, `v2`, `v3`, …

### Why URL-path versioning?

| Approach | Pros | Cons |
|---|---|---|
| **URL path** (chosen) | Explicit, cacheable, visible in logs | URLs change between versions |
| Accept header | Cleaner URLs | Harder to test in a browser; cache-unfriendly |
| Custom header | Flexible | Easy to forget; not visible in standard proxies |

URL-path versioning is the most widely understood convention and works cleanly
with all reverse proxies, load balancers, and API gateways.

### Optional header-based versioning

Two opt-in alternatives are available, configured via `VersioningConfig.strategy`
in `operator-config.yaml`:

```yaml
versioning:
  strategy: accept_header       # application/vnd.stellar.vN+json
  # or
  strategy: custom_header
  header_name: "X-API-Version"  # X-API-Version: v2
```

Header-based versioning is intended for clients that cannot control URL paths
(e.g., SDK auto-generated clients).  URL-path versioning remains the default.

---

## Version Lifecycle

```
Current  ──►  Deprecated  ──►  Sunset (410 Gone)
```

| State      | Served? | Minimum notice period |
|------------|---------|----------------------|
| Current    | ✅       | —                    |
| Deprecated | ✅       | 6 months             |
| Sunset     | ❌ 410   | Published in advance |

### Deprecation Headers (RFC 8594)

When a client calls a deprecated version, every response includes:

```
Deprecation: true
Sunset: Wed, 31 Dec 2026 00:00:00 GMT
Link: </api/v2>; rel="successor-version"
```

- `Deprecation` — signals that the version is deprecated.
- `Sunset` — the date on which the version will stop being served.
- `Link` — direct link to the successor version.

### Sunset (410 Gone)

After the sunset date, requests to the old version return:

```
HTTP/1.1 410 Gone
Sunset: Wed, 31 Dec 2026 00:00:00 GMT
Content-Type: application/json

{
  "error": "API version v1 has been sunset. Migrate to /api/v2.",
  "migration_guide": "https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/api-versioning.md"
}
```

---

## Current Version Matrix

| Version | Status     | Sunset Date | Notes |
|---------|------------|-------------|-------|
| v0      | Sunset     | 2025-06-30  | No longer served |
| v1      | Deprecated | 2026-12-31  | All endpoints available; migrate to v2 |
| **v2**  | **Current**| —           | Recommended for all new integrations |

---

## v1 → v2 Migration Guide

### What changed in v2

| Area | v1 | v2 |
|------|----|----|
| Nodes endpoint | `GET /api/v1/nodes` | `GET /api/v2/nodes` |
| Node status | `status.phase` (string) | `status.conditions[]` (array, Kubernetes-style) |
| Error format | `{"error": "..."}` | `{"code": 404, "message": "...", "details": {}}` |
| Pagination | Query params `page` + `per_page` | Cursor-based `after` + `limit` |
| Health | `GET /api/v1/health` | `GET /healthz` (no version prefix) |

### Minimal migration checklist

- [ ] Replace all `/api/v1/` path prefixes with `/api/v2/`.
- [ ] Update error-handling code to parse the new `{"code", "message", "details"}` shape.
- [ ] Replace page/per_page pagination with cursor pagination (`after` + `limit`).
- [ ] Switch status checks from `status.phase` to `status.conditions[*].type == "Ready"`.

### SDK support

The official Rust SDK (`stellar-k8s-client`) supports both v1 and v2 via a
compile-time feature flag:

```toml
# Cargo.toml
stellar-k8s-client = { version = "0.4", features = ["api-v2"] }
```

The `api-v2` feature is enabled by default as of `stellar-k8s-client 0.4.0`.

---

## Coexistence & Routing

Both `v1` and `v2` are served simultaneously during the deprecation window.
The gateway routes requests by inspecting the first `vN` path segment:

```
/api/v1/nodes  →  v1 handler  (+ deprecation headers)
/api/v2/nodes  →  v2 handler
```

No shared state is modified by v1 handlers — the versions are fully isolated
at the handler level.

---

## Operator Configuration

```yaml
# config/operator-config.yaml (or Helm values)
versioning:
  current_version: "v2"
  deprecated_versions:
    - "v1"
  sunset_versions:
    - "v0"
  sunset_dates:
    v1: "2026-12-31"
    v0: "2025-06-30"
  strategy: url_path   # default; also: accept_header, custom_header
```

---

## Adding a New API Version

1. Create `src/rest_api/v{N}/` with handler modules.
2. Register routes with the `v{N}` prefix in `src/rest_api/mod.rs`.
3. Add `v{N-1}` to `deprecated_versions` in `config/operator-config.yaml`.
4. Set `v{N-1}` sunset date (6 months minimum from the release of `v{N}`).
5. Update `docs/api/openapi.yaml` with both version paths.
6. Update this document.

---

## Verification

```bash
# Should return 200 with Deprecation headers
curl -I https://localhost:9090/api/v1/health

# Should return 200 without deprecation headers
curl -I https://localhost:9090/api/v2/nodes

# Should return 410 Gone
curl -I https://localhost:9090/api/v0/nodes
```
