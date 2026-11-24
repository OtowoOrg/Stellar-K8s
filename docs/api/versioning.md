# Operator REST API Versioning

This document defines the versioning strategy for the Stellar-K8s **operator REST API**
(`src/rest_api`). It satisfies GitHub issue **#1333**.

---

## Versioning strategy (decision)

**Canonical scheme: URL path**

```text
/api/v1/...
/api/v2/...
```

### Why URL path (not header-based)

1. Production routes already use `/api/v1/...` in `src/rest_api/server.rs`.
2. Path versions are visible in logs, traces, OpenAPI paths, and curl examples.
3. A single scheme avoids ambiguous conflicts between path and `Accept` /
   `X-API-Version` negotiation on the operator API.
4. Header negotiation helpers under `api_gateway` / `rest_api/gateway` remain for
   that separate proxy surface; they are **not** the operator REST contract.

Header-based version selection is **not** required for clients of the operator REST API.

### Introducing a future version

1. Add handlers under `/api/v2/...` beside existing `/api/v1/...` routes.
2. Set `REST_API_CURRENT_VERSION=v2`.
3. Mark the previous id deprecated (see below) so responses carry `Deprecation` /
   `Sunset` while both mounts continue to work.
4. Update OpenAPI and this document with migration notes for changed fields.

---

## Lifecycle model

```text
Current  -->  Deprecated  -->  Sunset (retired in catalog)
```

| Status | Served? | Client signal |
|--------|---------|---------------|
| **Current** | Yes | No deprecation headers |
| **Deprecated** | Yes (coexistence) | `Deprecation: true` and optional `Sunset: <HTTP-date>` |
| **Sunset** | Not mounted once removed; listed as retired in `/api/versions` | Catalog `status: sunset` |

HTTP **410 Gone** is intentionally **not** forced by the operator REST middleware.
Sunset is communicated through documentation, the version catalog, and headers while
a deprecated mount remains available.

---

## Coexistence

Multiple `/api/vN` trees can be registered on the same server. Today the shipped
surface is **`/api/v1`** (current). The router and middleware already understand
any `/api/vN` prefix so a future `/api/v2` can coexist without renaming clients.

Discovery:

```bash
curl -s https://operator:9090/api/versions
```

Example (default deployment):

```json
{
  "canonical_scheme": "url_path",
  "current": "v1",
  "versions": [
    {
      "id": "v1",
      "status": "current",
      "base_path": "/api/v1"
    }
  ]
}
```

Legacy paths such as `/v1/health/summary` (not under `/api/vN`) are outside this
versioning middleware and are not marked deprecated by it.

Unauthenticated probes (`/healthz`, `/readyz`, `/livez`, `/health`) and `/metrics`
are unversioned and never receive deprecation headers.

---

## Deprecation and Sunset headers

When a path version is configured as deprecated (or sunset-listed while still
routed), successful responses include:

| Header | Meaning |
|--------|---------|
| `Deprecation` | `true` - this API version is deprecated (RFC 8594 style) |
| `Sunset` | Optional HTTP-date when the version is planned for retirement |

Non-deprecated current versions must **not** include these headers.

### Configuration (explicit; no unexplained hardcoded dates)

| Environment variable | Purpose | Example |
|----------------------|---------|---------|
| `REST_API_CURRENT_VERSION` | Current version id | `v1` (default) |
| `REST_API_DEPRECATED_VERSIONS` | Comma-separated deprecated ids | `v1` |
| `REST_API_SUNSET_VERSIONS` | Comma-separated retired ids (catalog) | `v0` |
| `REST_API_SUNSET_DATES` | Per-version HTTP-dates | `v1=Wed, 01 Sep 2027 00:00:00 GMT` |

Implementation: `src/rest_api/versioning.rs` (middleware on the operator router).

---

## Migration path

```text
v1 client
   |
   v
GET /api/versions  (confirm current vs deprecated)
   |
   v
Watch for Deprecation / Sunset on /api/v1 responses
   |
   v
Read migration notes for /api/v2 (when published)
   |
   v
Point clients at /api/v2/... equivalents
   |
   v
Validate auth, payloads, and CI/OpenAPI
   |
   v
Retire v1 usage before the published Sunset date
```

### Concrete examples (today)

| Task | Path |
|------|------|
| List nodes | `GET /api/v1/nodes` |
| Get node | `GET /api/v1/nodes/{namespace}/{name}` |
| Dashboard overview | `GET /api/v1/dashboard/overview` |
| Jobs | `GET /api/v1/jobs` |
| Version catalog | `GET /api/versions` |

When `/api/v2` is introduced, prefer the v2 paths listed in the updated OpenAPI
document and keep calling `/api/versions` during the overlap window.

---

## Related surfaces

| Surface | Versioning |
|---------|------------|
| Operator REST (`rest_api`) | **This document** - URL path `/api/vN` |
| `api_gateway` crate module | Separate proxy/gateway config (`VersioningConfig`) |
| Kubernetes CRDs | [ADR-0004](../adr/0004-crd-versioning-strategy.md) (not REST) |
| custom.metrics.k8s.io | Kubernetes API group versions (not `/api/vN`) |

---

## See also

- [API index](index.md)
- [OpenAPI specification](openapi.yaml)
- [Client libraries](client-libraries.md)
