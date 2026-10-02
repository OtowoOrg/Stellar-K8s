# soroban-audit-sample: deliberately vulnerable, do not deploy

A small Soroban `Vault` + `Lending` contract pair with **injected
vulnerabilities**. It is used to validate the
[Soroban audit framework](../../docs/security/smart-contract-audit.md) and its
[checklist template](../../docs/templates/audit-checklist.md).

The crate is built as `rlib` only, so it can't be turned into deployable WASM
by accident.

| Injected bug | Location | Checklist item | Finding |
|---|---|---|---|
| Authorization bypass (`to.require_auth()` instead of `from`) | `src/vault.rs:55` | AUTH-02 | F-01 |
| Cross-contract reentrancy (hook called before effects) | `src/vault.rs:79-91` | XCC-02 | F-02 |

- [`AUDIT.md`](AUDIT.md): the completed checklist and findings.
- [`src/test.rs`](src/test.rs): exploit tests (they **pass** because the bugs
  are real), control tests, and a test showing the host rejects direct
  re-entry.
- [`fuzz/`](fuzz/): `cargo-fuzz` invariant targets that find both bugs
  without being told where they are.

## Run

```bash
cargo test

# fuzzing: nightly toolchain + `cargo install cargo-fuzz --locked`
cd fuzz
cargo +nightly fuzz run auth_invariant     -- -max_total_time=300
cargo +nightly fuzz run solvency_invariant -- -max_total_time=300
# macOS: if libFuzzer aborts at startup under AddressSanitizer, add `-s none`
```

Both fuzz targets are **expected to crash**. That is how they report each
bug.
