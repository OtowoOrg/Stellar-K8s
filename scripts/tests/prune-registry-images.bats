#!/usr/bin/env bats
#
# bats suite for scripts/prune-registry-images.sh
#
# Covers:
#   - argument validation (exit code 2) and --help
#   - dry run is the default and never deletes
#   - protected tags, repo-referenced tags/digests, child manifests and cosign
#     attachments are never selected for deletion
#   - --min-age-days, --keep-latest and --max-delete limits
#   - the live path (gh and curl shims): the plan is printed before anything is
#     deleted, exactly the planned versions are deleted, and a failed registry
#     lookup deletes nothing
#
# The offline fixtures live in scripts/tests/fixtures/prune-registry-images/.
# FIXTURE_NOW is 2026-11-23T00:00:00Z; every age in the fixture is relative to it.

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  SCRIPT="${REPO_ROOT}/scripts/prune-registry-images.sh"
  FIXTURES="${BATS_TEST_DIRNAME}/fixtures/prune-registry-images"
  FIXTURE_NOW=1795392000

  TEST_TMP="$(mktemp -d)"

  # A tiny repo tree with the image references the script must respect.
  TREE="${TEST_TMP}/tree"
  mkdir -p "${TREE}/charts/stellar-operator/tests" "${TREE}/config/manifests" "${TREE}/examples"
  cat >"${TREE}/charts/stellar-operator/Chart.yaml" <<'EOF'
apiVersion: v2
name: stellar-operator
version: 7.7.7
appVersion: "7.7.7"
EOF
  cat >"${TREE}/charts/stellar-operator/values.yaml" <<'EOF'
image:
  repository: ghcr.io/otowoorg/stellar-k8s
  pullPolicy: IfNotPresent
  tag: "pinned-in-values"
sidecar:
  image:
    repository: ghcr.io/otowoorg/stellar-k8s
    tag: ""
EOF
  # tests/ directories hold render fixtures, not deployments: ignored.
  echo 'image: ghcr.io/otowoorg/stellar-k8s:ignored-test-fixture-tag' \
    >"${TREE}/charts/stellar-operator/tests/render.yaml"
  PINNED_DIGEST="$(jq -r '.[] | select(.id == 1010) | .name' "${FIXTURES}/versions.json")"
  cat >"${TREE}/config/manifests/deploy.yaml" <<EOF
spec:
  containers:
    - image: ghcr.io/otowoorg/stellar-k8s:custom-pin
    - image: "ghcr.io/otowoorg/stellar-k8s@${PINNED_DIGEST}"
    - image: ghcr.io/otowoorg/stellar-k8s-other:not-ours
EOF
  cat >"${TREE}/examples/kustomization.yaml" <<'EOF'
images:
  - name: ghcr.io/otowoorg/stellar-k8s
    newTag: staging-2026-07
EOF

  SHIM_BIN="${TEST_TMP}/bin"
  CALLS="${TEST_TMP}/gh-delete-calls.txt"
  mkdir -p "${SHIM_BIN}"
  : >"${CALLS}"
  ORIGINAL_PATH="${PATH}"
}

teardown() {
  export PATH="${ORIGINAL_PATH}"
  rm -rf "${TEST_TMP}"
}

# Offline dry run against the fixture, with the limits the expectations assume.
offline() {
  bash "${SCRIPT}" \
    --versions-file "${FIXTURES}/versions.json" \
    --manifests-file "${FIXTURES}/manifests.json" \
    --repo-root "${TREE}" --now "${FIXTURE_NOW}" \
    --keep-latest 3 --min-age-days 30 "$@"
}

# assert_action <version id> <KEEP|DELETE> [reason]  (reads $output)
assert_action() {
  local row
  row="$(grep -E "^(KEEP|DELETE) +$1 " <<<"${output}")" || { echo "no plan row for version $1"; return 1; }
  [[ "${row%% *}" == "$2" ]] || { echo "version $1: expected $2, got: ${row}"; return 1; }
  if [[ -n "${3:-}" ]]; then
    [[ "${row}" == *" $3 "* ]] || { echo "version $1: expected reason $3, got: ${row}"; return 1; }
  fi
}

# gh and curl stand-ins for the live path. gh serves the fixture and records
# DELETE calls; curl serves a registry token and index manifests.
install_shims() {
  cat >"${SHIM_BIN}/gh" <<EOF
#!/usr/bin/env bash
args="\$*"
if [[ "\$args" == *"--method DELETE"* ]]; then
  echo "\${args##* }" >>"${CALLS}"
  exit 0
fi
if [[ "\$args" == *"orgs/OtowoOrg/packages/container/stellar-k8s/versions"* ]]; then
  cat "${FIXTURES}/versions.json"
  exit 0
fi
echo "unexpected gh call: \$args" >&2
exit 1
EOF
  cat >"${SHIM_BIN}/curl" <<EOF
#!/usr/bin/env bash
[[ "\$*" == *"--config -"* ]] && cat >/dev/null   # credentials arrive on stdin
url="\${*: -1}"
if [[ "\${REGISTRY_FAIL:-}" == "1" && "\$url" == *"/manifests/"* ]]; then
  printf '{"errors":[]}\n500'
  exit 0
fi
if [[ "\$url" == *"/token?"* ]]; then
  echo '{"token":"test-token"}'
  exit 0
fi
digest="\${url##*/manifests/}"
jq -c --arg d "\$digest" '
  if has(\$d) then {schemaVersion: 2, manifests: [.[\$d][] | {digest: .}]} else {schemaVersion: 2, config: {}} end' \
  "${FIXTURES}/manifests.json"
printf '\n200'
EOF
  chmod +x "${SHIM_BIN}/gh" "${SHIM_BIN}/curl"
  export PATH="${SHIM_BIN}:${PATH}"
  export GH_TOKEN="test-token"
}

@test "prune-registry-images.sh exists and is executable" {
  [ -f "${SCRIPT}" ]
  [ -x "${SCRIPT}" ]
}

@test "--help exits 0 and documents the dry-run default" {
  run bash "${SCRIPT}" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"Usage:"* ]]
  [[ "$output" == *"dry run (the default)"* ]]
  [[ "$output" == *"--delete"* ]]
}

@test "unknown arguments are rejected with exit 2" {
  run bash "${SCRIPT}" --nope
  [ "$status" -eq 2 ]
  [[ "$output" == *"unknown argument"* ]]
}

@test "--min-age-days below 1 is rejected" {
  run bash "${SCRIPT}" --min-age-days 0 --versions-file "${FIXTURES}/versions.json"
  [ "$status" -eq 2 ]
  [[ "$output" == *"at least 1"* ]]
}

@test "--delete cannot be combined with a fixture file" {
  run bash "${SCRIPT}" --delete --versions-file "${FIXTURES}/versions.json"
  [ "$status" -eq 2 ]
  [[ "$output" == *"never deleted"* ]]
}

@test "dry run is the default: prints the plan and deletes nothing" {
  install_shims
  run offline
  [ "$status" -eq 0 ]
  [[ "$output" == *"mode:          DRY RUN"* ]]
  [[ "$output" == *"Dry run: no versions were deleted"* ]]
  [[ "$output" == *"Summary: keep 17, delete 6 of 23 version(s)"* ]]
  [ ! -s "${CALLS}" ]
}

@test "protected tags are kept even when every limit is as aggressive as allowed" {
  run offline --keep-latest 0 --min-age-days 1
  [ "$status" -eq 0 ]
  assert_action 1001 KEEP protected-tag   # latest, main
  assert_action 1006 KEEP protected-tag   # 4.0.0, 4.0, 4
  assert_action 1014 KEEP protected-tag   # v3.9.1
  assert_action 1015 KEEP protected-tag   # 1.2.3-rc.1
}

@test "--protect-tag adds a custom protected tag pattern" {
  run offline --protect-tag '^experimental-'
  [ "$status" -eq 0 ]
  assert_action 1020 KEEP protected-tag
}

@test "tags and digests referenced by the repo are kept" {
  run offline --keep-latest 0 --min-age-days 1
  [ "$status" -eq 0 ]
  assert_action 1009 KEEP referenced-in-repo     # manifest image: ...:custom-pin
  assert_action 1011 KEEP referenced-in-repo     # kustomize newTag
  assert_action 1010 KEEP referenced-digest      # image: ...@sha256:<digest>
}

@test "references under tests/ directories do not protect anything" {
  run offline
  [ "$status" -eq 0 ]
  assert_action 1023 DELETE stale-tag
}

@test "child manifests of kept images are kept, including nested children" {
  run offline
  [ "$status" -eq 0 ]
  assert_action 1012 KEEP child-of-kept-image
  assert_action 1013 KEEP child-of-kept-image
  assert_action 1022 KEEP child-of-kept-image   # child of 1012, itself a child of 4.0.0
  assert_action 1021 KEEP child-of-kept-image   # child of a recent index
}

@test "without the manifests file the same untagged versions are pruned" {
  run bash "${SCRIPT}" --versions-file "${FIXTURES}/versions.json" \
    --repo-root "${TREE}" --now "${FIXTURE_NOW}" --keep-latest 3 --min-age-days 30
  [ "$status" -eq 0 ]
  assert_action 1012 DELETE stale-untagged
}

@test "cosign attachments follow the fate of the image they belong to" {
  run offline
  [ "$status" -eq 0 ]
  assert_action 1017 KEEP attached-to-kept-image      # .sig of 4.0.0
  assert_action 1018 KEEP attached-to-kept-image      # attestation of 4.0.0
  assert_action 1016 DELETE attached-to-pruned-image  # .sig of a pruned per-commit image
  assert_action 1019 KEEP attachment-subject-unknown  # never delete what we cannot place
}

@test "recent and young versions are kept, stale ones are planned for deletion" {
  run offline
  [ "$status" -eq 0 ]
  assert_action 1002 KEEP recent
  assert_action 1003 KEEP recent
  assert_action 1004 KEEP too-young
  assert_action 1005 DELETE stale-tag
  assert_action 1007 DELETE stale-untagged
  assert_action 1008 DELETE stale-tag
  assert_action 1020 DELETE stale-tag
}

@test "--min-age-days larger than every version keeps everything" {
  run offline --min-age-days 400
  [ "$status" -eq 0 ]
  [[ "$output" == *"delete 0 of 23"* ]]
}

@test "--keep-latest larger than the package keeps everything" {
  run offline --keep-latest 100
  [ "$status" -eq 0 ]
  [[ "$output" == *"delete 0 of 23"* ]]
}

@test "--delete refuses a plan larger than --max-delete and deletes nothing" {
  install_shims
  run bash "${SCRIPT}" --delete --repo-root "${TREE}" --now "${FIXTURE_NOW}" \
    --keep-latest 3 --min-age-days 30 --max-delete 2
  [ "$status" -eq 3 ]
  [[ "$output" == *"refusing to continue"* ]]
  [ ! -s "${CALLS}" ]
}

@test "--delete prints the plan first, then deletes exactly the planned versions" {
  install_shims
  run bash "${SCRIPT}" --delete --repo-root "${TREE}" --now "${FIXTURE_NOW}" \
    --keep-latest 3 --min-age-days 30
  [ "$status" -eq 0 ]
  [[ "$output" == *"mode:          DELETE"* ]]
  [[ "$output" == *"Deleted 6 version(s), 0 failure(s)"* ]]
  # The plan is printed before the first deletion.
  plan_line="$(grep -n 'Registry prune plan' <<<"${output}" | head -1 | cut -d: -f1)"
  first_delete_line="$(grep -n '^deleted version' <<<"${output}" | head -1 | cut -d: -f1)"
  [ "${plan_line}" -lt "${first_delete_line}" ]
  # Exactly the six planned versions, and nothing else, were deleted.
  sorted_calls="$(sed 's#.*/versions/##' "${CALLS}" | sort | tr '\n' ' ')"
  [ "${sorted_calls}" = "1005 1007 1008 1016 1020 1023 " ]
}

@test "--delete deletes nothing when a registry lookup fails" {
  install_shims
  REGISTRY_FAIL=1 run bash "${SCRIPT}" --delete --repo-root "${TREE}" --now "${FIXTURE_NOW}" \
    --keep-latest 3 --min-age-days 30
  [ "$status" -ne 0 ]
  [[ "$output" == *"registry lookup failed"* ]]
  [ ! -s "${CALLS}" ]
}

@test "live dry run resolves children through the registry and deletes nothing" {
  install_shims
  run bash "${SCRIPT}" --repo-root "${TREE}" --now "${FIXTURE_NOW}" --keep-latest 3 --min-age-days 30
  [ "$status" -eq 0 ]
  assert_action 1012 KEEP child-of-kept-image
  assert_action 1022 KEEP child-of-kept-image
  [[ "$output" == *"Dry run: no versions were deleted"* ]]
  [ ! -s "${CALLS}" ]
}

@test "scan_references collects repo references and skips tests/ and other images" {
  run bash -c "source '${SCRIPT}'; scan_references '${TREE}' stellar-k8s | jq -c ."
  [ "$status" -eq 0 ]
  [[ "$output" == *'"7.7.7"'* ]]
  [[ "$output" == *'"v7.7.7"'* ]]
  [[ "$output" == *'"custom-pin"'* ]]
  [[ "$output" == *'"pinned-in-values"'* ]]
  [[ "$output" == *'"staging-2026-07"'* ]]
  [[ "$output" == *"${PINNED_DIGEST}"* ]]
  [[ "$output" != *"ignored-test-fixture-tag"* ]]
  [[ "$output" != *"not-ours"* ]]
}
