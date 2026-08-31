#!/usr/bin/env bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
# scripts/lib/health-steps.sh
# Shared repository health check steps used by repo-health.sh.

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

sk8s_health_stale_docs() {
  cargo run --bin doc-check -- --warn-only
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
  bash "${REPO_ROOT}/scripts/dep-gate.sh" --audit-only
}

sk8s_health_helm_lint() {
  helm lint charts/stellar-operator
}

sk8s_health_issue_templates() {
  python3 scripts/issue_template_lint.py
}

