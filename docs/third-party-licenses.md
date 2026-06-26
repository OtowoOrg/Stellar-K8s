# Third-Party License References

This document tracks how to audit third-party dependency licenses for Stellar-K8s.

## Audit Process

1. Run `make license-audit` from the repository root.
2. Review `UNKNOWN` entries in the generated table.
3. Resolve missing license metadata before release.
4. Re-run after any `Cargo.lock` update.

## Generation Command

```bash
make license-audit
```

The `license-audit` target uses [scripts/audit-licenses.sh](../scripts/audit-licenses.sh), which reads dependency metadata with:

```bash
cargo metadata --format-version 1 --locked
```

## Notes

- This repository is licensed under Apache-2.0 (see [LICENSE](../LICENSE)).
- Third-party crate licenses are declared by upstream crates and should be verified for compatibility.
- CI or release automation should run `make license-audit` in an environment where `cargo` is available.
