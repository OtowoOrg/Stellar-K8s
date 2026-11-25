#!/usr/bin/env bats
#
# bats suite for scripts/apply-branch-protection.sh and .github/rulesets/main.json
#
# Covers:
#   - the committed ruleset: PR required with approvals, required status checks,
#     force-push and deletion blocked, direct pushes restricted
#   - every required check name is a real job name in .github/workflows
#   - dry run is the default and never writes (gh is replaced by a shim)
#   - --apply creates or updates by name, and refuses a check no workflow reports
#   - --check reports a missing, differing or matching live ruleset
#

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  SCRIPT="${REPO_ROOT}/scripts/apply-branch-protection.sh"
  RULESET="${REPO_ROOT}/.github/rulesets/main.json"

  TEST_TMP="$(mktemp -d)"
  SHIM_BIN="${TEST_TMP}/bin"
  mkdir -p "${SHIM_BIN}"
  export GH_CALLS="${TEST_TMP}/gh-calls.txt"
  export GH_LIST_JSON="${TEST_TMP}/list.json"
  export GH_LIVE_JSON="${TEST_TMP}/live.json"
  : >"${GH_CALLS}"
  echo '[]' >"${GH_LIST_JSON}"
  ORIGINAL_PATH="${PATH}"

  cat >"${SHIM_BIN}/gh" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"${GH_CALLS}"
case "$*" in
  *"--method POST"* | *"--method PUT"*) exit 0 ;;
  *"api --paginate repos/"*"/rulesets")
    if [[ "${GH_LIST_FAIL:-}" == "1" ]]; then echo "HTTP 401: Requires authentication" >&2; exit 1; fi
    cat "${GH_LIST_JSON}" ;;
  *"api repos/"*"/rulesets/"*) cat "${GH_LIVE_JSON}" ;;
  *) echo "unexpected gh call: $*" >&2; exit 1 ;;
esac
EOF
  chmod +x "${SHIM_BIN}/gh"
  export PATH="${SHIM_BIN}:${PATH}"
}

teardown() {
  export PATH="${ORIGINAL_PATH}"
  rm -rf "${TEST_TMP}"
}

# writes_made: any POST or PUT recorded by the gh shim
writes_made() { grep -qE -- '--method (POST|PUT)' "${GH_CALLS}"; }

# A live ruleset as GitHub returns it: extra fields, defaulted parameters,
# and the required checks in a different order.
make_live_ruleset() {
  jq '. + {id: 42, source_type: "Repository", node_id: "RRS_x", current_user_can_bypass: "never"}
      | (.rules[] | select(.type == "pull_request") | .parameters) += {
          required_reviewers: [], allowed_merge_methods: ["merge", "squash", "rebase"]}
      | (.rules[] | select(.type == "required_status_checks") | .parameters.required_status_checks) |= reverse' \
    "${RULESET}" >"${GH_LIVE_JSON}"
  echo '[{"id": 42, "name": "main-protection"}]' >"${GH_LIST_JSON}"
}

# ── the committed ruleset ────────────────────────────────────────────────────

@test "ruleset file is valid JSON for a branch ruleset on main" {
  run jq -e '.name == "main-protection" and .target == "branch" and .enforcement == "active"
             and (.conditions.ref_name.include == ["refs/heads/main"])' "${RULESET}"
  [ "$status" -eq 0 ]
}

@test "ruleset requires pull requests with at least one approval" {
  run jq -e '[.rules[] | select(.type == "pull_request")] | length == 1
             and .[0].parameters.required_approving_review_count >= 1' "${RULESET}"
  [ "$status" -eq 0 ]
}

@test "ruleset requires status checks reported by GitHub Actions" {
  run jq -e '[.rules[] | select(.type == "required_status_checks")] | length == 1
             and (.[0].parameters.required_status_checks | length > 0)
             and (.[0].parameters.required_status_checks | all(.integration_id == 15368))' "${RULESET}"
  [ "$status" -eq 0 ]
}

@test "ruleset blocks force pushes and deletion of main" {
  run jq -e '[.rules[].type] | index("non_fast_forward") != null and index("deletion") != null' "${RULESET}"
  [ "$status" -eq 0 ]
}

@test "direct pushes are restricted: PR rule applies and bypass is pull-request-only" {
  run jq -e '(.bypass_actors | all(.bypass_mode == "pull_request"))
             and ([.rules[].type] | index("pull_request") != null)' "${RULESET}"
  [ "$status" -eq 0 ]
}

@test "every required check name is a job name in .github/workflows" {
  run bash -c "source '${SCRIPT}'; unknown_checks '${RULESET}' '${REPO_ROOT}/.github/workflows'"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "unknown_checks reports a check no workflow defines" {
  jq '(.rules[] | select(.type == "required_status_checks") | .parameters.required_status_checks)
        += [{context: "No Such Job", integration_id: 15368}]' "${RULESET}" >"${TEST_TMP}/bad.json"
  run bash -c "source '${SCRIPT}'; unknown_checks '${TEST_TMP}/bad.json' '${REPO_ROOT}/.github/workflows'"
  [ "$status" -eq 0 ]
  [ "$output" = "No Such Job" ]
}

# ── the script ───────────────────────────────────────────────────────────────

@test "apply-branch-protection.sh exists and is executable" {
  [ -f "${SCRIPT}" ]
  [ -x "${SCRIPT}" ]
}

@test "--help exits 0 and documents dry run, --apply and --check" {
  run bash "${SCRIPT}" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"dry run (the default)"* ]]
  [[ "$output" == *"--apply"* ]]
  [[ "$output" == *"--check"* ]]
}

@test "bad usage exits 2" {
  run bash "${SCRIPT}" --nope
  [ "$status" -eq 2 ]
  run bash "${SCRIPT}" --apply --check
  [ "$status" -eq 2 ]
  run bash "${SCRIPT}" --repo not-a-repo
  [ "$status" -eq 2 ]
}

@test "dry run is the default: prints the plan and payload, writes nothing" {
  run bash "${SCRIPT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"DRY RUN (nothing will be sent)"* ]]
  [[ "$output" == *"Would create ruleset \"main-protection\": POST repos/OtowoOrg/Stellar-K8s/rulesets"* ]]
  [[ "$output" == *"- Detect Changes"* ]]
  [[ "$output" == *"- Validate Conventional Commits"* ]]
  [[ "$output" == *'"required_approving_review_count": 1'* ]]
  [[ "$output" == *"nothing was sent"* ]]
  ! writes_made
}

@test "dry run says update when the ruleset already exists" {
  make_live_ruleset
  run bash "${SCRIPT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Would update ruleset \"main-protection\" (id 42): PUT repos/OtowoOrg/Stellar-K8s/rulesets/42"* ]]
  ! writes_made
}

@test "dry run still works when gh cannot list rulesets" {
  GH_LIST_FAIL=1 run bash "${SCRIPT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Would create or update"* ]]
  ! writes_made
}

@test "--repo targets another repository" {
  run bash "${SCRIPT}" --repo Example/Fork
  [ "$status" -eq 0 ]
  [[ "$output" == *"POST repos/Example/Fork/rulesets"* ]]
}

@test "--apply creates the ruleset when it does not exist" {
  run bash "${SCRIPT}" --apply
  [ "$status" -eq 0 ]
  [[ "$output" == *"Created ruleset \"main-protection\""* ]]
  grep -qE -- "--method POST repos/OtowoOrg/Stellar-K8s/rulesets --input ${RULESET}" "${GH_CALLS}"
  ! grep -q -- '--method PUT' "${GH_CALLS}"
}

@test "--apply updates the existing ruleset in place" {
  make_live_ruleset
  run bash "${SCRIPT}" --apply
  [ "$status" -eq 0 ]
  [[ "$output" == *"Updated ruleset \"main-protection\" (id 42)"* ]]
  grep -qE -- "--method PUT repos/OtowoOrg/Stellar-K8s/rulesets/42 --input ${RULESET}" "${GH_CALLS}"
  ! grep -q -- '--method POST' "${GH_CALLS}"
}

@test "--apply refuses a required check that no workflow reports" {
  jq '(.rules[] | select(.type == "required_status_checks") | .parameters.required_status_checks)
        += [{context: "No Such Job", integration_id: 15368}]' "${RULESET}" >"${TEST_TMP}/bad.json"
  run bash "${SCRIPT}" --apply --ruleset "${TEST_TMP}/bad.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"No Such Job"* ]]
  [[ "$output" == *"refusing to apply"* ]]
  ! writes_made
}

@test "--apply stops when rulesets cannot be listed" {
  GH_LIST_FAIL=1 run bash "${SCRIPT}" --apply
  [ "$status" -ne 0 ]
  [[ "$output" == *"could not list rulesets"* ]]
  ! writes_made
}

@test "--check fails when the ruleset is missing" {
  run bash "${SCRIPT}" --check
  [ "$status" -eq 1 ]
  [[ "$output" == *"MISSING"* ]]
}

@test "--check passes when the live ruleset matches despite extra fields and ordering" {
  make_live_ruleset
  run bash "${SCRIPT}" --check
  [ "$status" -eq 0 ]
  [[ "$output" == *"OK: ruleset \"main-protection\" (id 42) is present"* ]]
}

@test "--check reports every way the live ruleset falls short" {
  make_live_ruleset
  jq '.enforcement = "disabled"
      | .rules |= map(select(.type != "non_fast_forward"))
      | (.rules[] | select(.type == "pull_request") | .parameters.required_approving_review_count) = 0
      | (.rules[] | select(.type == "required_status_checks") | .parameters.required_status_checks) |= map(select(.context != "Codespell"))' \
    "${GH_LIVE_JSON}" >"${TEST_TMP}/live2.json"
  mv "${TEST_TMP}/live2.json" "${GH_LIVE_JSON}"
  run bash "${SCRIPT}" --check
  [ "$status" -eq 1 ]
  [[ "$output" == *"DIFFERS"* ]]
  [[ "$output" == *"enforcement is disabled, want active"* ]]
  [[ "$output" == *"rule non_fast_forward is missing"* ]]
  [[ "$output" == *"required_approving_review_count is 0, want 1"* ]]
  [[ "$output" == *"required_status_checks"* ]]
}
