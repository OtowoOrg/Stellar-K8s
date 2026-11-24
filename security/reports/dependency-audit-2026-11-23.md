# Dependency Vulnerability Scan — 2026-11-23

Scan tool: `cargo audit` (cargo-audit 0.22.2) against `Cargo.lock` (711 crate dependencies), RustSec advisory database (1562 advisories).

## Result

- **22 vulnerabilities** found (unpatched or requiring a version bump not yet applied).
- **18 additional advisory warnings** (unmaintained/unsound, allowed by current `deny.toml` policy).

## Critical / notable findings

| Crate | Version | Advisory | Issue |
|---|---|---|---|
| `libsqlite3-sys` | 0.30.1 | RUSTSEC (CVE-2026-119375) | Heap buffer overflow via bundled SQLite |
| `wasmtime` / `wasmtime-wasi` | 24.0.13 | multiple RUSTSEC advisories | Unbounded host memory growth, WASI sandbox escape on Windows, Cranelift miscompilation |
| `zip` | 2.6.1 | multiple RUSTSEC advisories | AES entries accepted without authentication check; local/central-directory name mismatch |
| `rustls-webpki` | 0.101.7, 0.103.15 | RUSTSEC | Certificate verification may accept distrusted intermediates |

The remaining findings are lower-impact `unsound`/timing-variability advisories in `crypto-bigint`, `ctutils`, `libz-sys`, `pqcrypto-dilithium`, `url`, `zstd`/`zstd-safe`. Full advisory IDs and descriptions are reproducible via `cargo audit` (see `make audit` / `make audit-quick`).

## Next steps

- Track remediation per advisory via `cargo update -p <crate>` where a fixed version is published upstream.
- Re-run `cargo audit` as part of `make security-report` after dependency bumps.
