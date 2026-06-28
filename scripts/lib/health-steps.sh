#!/usr/bin/env bash
# scripts/lib/health-steps.sh
# Shared repository health check steps used by repo-health.sh and validate.sh.

: "${REPO_ROOT:?REPO_ROOT must be set before sourcing health-steps.sh}"

K8S_OPENAPI_ENABLED_VERSION="${K8S_OPENAPI_ENABLED_VERSION:-1.30}"
export K8S_OPENAPI_ENABLED_VERSION

readonly SK8S_CARGO_FEATURES='rest-api,metrics,admission-webhook,k8s-v1-30,reconciler-fuzz'

readonly -a SK8S_CLIPPY_DENY=(
  -D clippy::correctness
  -D clippy::suspicious
  -D clippy::perf
  -D clippy::style
)

readonly -a SK8S_CLIPPY_ALLOW=(
  -A clippy::new_without_default
  -A clippy::match_like_matches_macro
  -A clippy::match_result_ok
  -A clippy::needless_borrow
  -A clippy::get_first
  -A clippy::format_in_format_args
  -A clippy::single_match
  -A clippy::redundant_closure
  -A clippy::items_after_test_module
  -A clippy::approx_constant
  -A clippy::should_implement_trait
)

sk8s_health_fmt_check() {
  cargo fmt --all --check
}

sk8s_health_clippy() {
  cargo clippy --workspace --all-targets --all-features -- \
    "${SK8S_CLIPPY_DENY[@]}" \
    "${SK8S_CLIPPY_ALLOW[@]}"
}

sk8s_health_lint_ci_features() {
  cargo clippy --workspace --all-targets \
    --features "${SK8S_CARGO_FEATURES}" -- \
    "${SK8S_CLIPPY_DENY[@]}" \
    "${SK8S_CLIPPY_ALLOW[@]}"
}

sk8s_health_test() {
  cargo test --workspace --features "${SK8S_CARGO_FEATURES}" --tests --lib --bins
}

sk8s_health_compile_check() {
  cargo test --workspace --no-run
}

sk8s_health_api_docs() {
  python3 scripts/generate-api-docs.py \
    --crd config/crd/stellarnode-crd.yaml \
    --output docs/api-reference.md \
    --check
}

sk8s_health_shellcheck() {
  mapfile -t shell_files < <(find scripts -name '*.sh' -type f | sort)
  if ((${#shell_files[@]} == 0)); then
    return 0
  fi
  shellcheck -S error "${shell_files[@]}"
}

sk8s_health_link_check() {
  python3 scripts/check-links.py
}

sk8s_health_cargo_audit() {
  if ! command -v cargo-audit >/dev/null 2>&1; then
    cargo install --locked cargo-audit
  fi
  cargo audit --deny unsound
}

sk8s_health_helm_lint() {
  helm lint charts/stellar-operator
}
