# Canonical Repository Health Checklist

Use this checklist before merging any Pull Request (PR) that touches code, scripts, manifests, or documentation. It represents the quality standard required to keep the Stellar-K8s repository healthy, clean, and navigable.

---

## 1. Code Quality & Compilation

- [ ] **Formatting**: Run `make fmt` locally to format all Rust code. Ensure `make fmt-check` passes.
- [ ] **Clippy Lints**: Run `make lint` to verify that there are no compiler warnings. All Clippy alerts must be clean.
- [ ] **Dependency Audit**: Run `make audit` to ensure there are no unreviewed or un-ignored security vulnerabilities in dependencies.
- [ ] **Unused Imports & Dead Code**:
  - Remove all unused imports in modified files.
  - Do not introduce new `#[allow(dead_code)]` attributes without adding an explanatory comment justifying why the code is currently unused.
- [ ] **Documentation Comments**: All new public functions, structs, enums, and modules must have proper Rust doc comments (`///` or `//!`).

---

## 2. Testing

- [ ] **Local Unit Tests**: Run `make test` (or `cargo test`) and confirm that all unit and integration tests pass successfully.
- [ ] **No Regression**: Ensure any modified logic is accompanied by updated or new unit tests validating the behavior.

---

## 3. Scripts & Manifests

- [ ] **Shellcheck**: All modified or new bash scripts in `scripts/` must pass `shellcheck -S error`. Sourcing `scripts/lib/errors.sh` is highly recommended for standard logs.
- [ ] **Script Naming**: Scripts must follow the `kebab-case.sh` naming convention.
- [ ] **Regenerate Manifests**: If your changes modify CRD definitions (in `src/crd/`), API docs generation scripts, or Helm templates, you must regenerate and commit the updated output files:
  - Run `make generate-api-docs` to regenerate `docs/api-reference.md`.
  - Ensure all Helm chart values and CRDs are up to date.

---

## 4. Documentation & Links

- [ ] **Relative Links**: Verify all links inside documentation files point to valid targets. Relative paths must exist.
- [ ] **Link Validation**: Run `python3 scripts/check-links.py` locally and ensure it reports zero broken links.
- [ ] **File Names**: New documentation files must follow the `kebab-case.md` naming convention.
- [ ] **Index Registration**: New documentation files must be registered under the correct category in `mkdocs.yml` navigation block to prevent dangling or orphaned pages.

---

## 5. Commits & PR Standards

- [ ] **Conventional Commits**: Commit messages must follow the [Conventional Commits](https://www.conventionalcommits.org/) specification (e.g., `feat: ...`, `fix: ...`, `docs: ...`).
- [ ] **DCO Sign-off**: Every commit must be signed off with the Developer Certificate of Origin (DCO) using `git commit -s` (resulting in a `Signed-off-by: Name <email>` footer).
