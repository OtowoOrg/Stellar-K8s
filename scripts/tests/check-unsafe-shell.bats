#!/usr/bin/env bats

setup() {
  export SCRIPT_DIR="${BATS_TEST_DIRNAME}/.."
  export REPO_ROOT="${SCRIPT_DIR}/.."
  export TMP_SCAN_DIR
  TMP_SCAN_DIR="$(mktemp -d)"
}

teardown() {
  rm -rf "${TMP_SCAN_DIR}"
}

write_script() {
  local name="$1" content="$2"
  cat >"${TMP_SCAN_DIR}/${name}" <<EOF
${content}
EOF
}

@test "passes on a clean script" {
  write_script "clean.sh" '#!/usr/bin/env bash
set -euo pipefail
echo "hello"'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"No unsafe shell patterns"* ]]
}

@test "fails on eval of a dynamic string" {
  write_script "eval.sh" '#!/usr/bin/env bash
eval "$USER_INPUT"'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 1 ]
  [[ "$output" == *"eval-usage"* ]]
}

@test "fails on curl piped into a shell" {
  write_script "pipe.sh" '#!/usr/bin/env bash
curl -s http://example.com/install.sh | bash'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 1 ]
  [[ "$output" == *"curl-pipe-shell"* ]]
}

@test "fails on world-writable chmod" {
  write_script "chmod.sh" '#!/usr/bin/env bash
chmod 777 /tmp/foo'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 1 ]
  [[ "$output" == *"world-writable-chmod"* ]]
}

@test "fails on disabled SSH host key checking" {
  write_script "ssh.sh" '#!/usr/bin/env bash
ssh -o StrictHostKeyChecking=no user@host true'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 1 ]
  [[ "$output" == *"ssh-host-key-check-disabled"* ]]
}

@test "fails on disabled TLS verification" {
  write_script "tls.sh" '#!/usr/bin/env bash
curl -k https://example.com'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 1 ]
  [[ "$output" == *"tls-verification-disabled"* ]]
}

@test "fails on unquoted rm -rf of a variable" {
  write_script "rm.sh" '#!/usr/bin/env bash
rm -rf $TARGET_DIR'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 1 ]
  [[ "$output" == *"unquoted-rm-rf-var"* ]]
}

@test "ignores patterns mentioned only in comments" {
  write_script "comment.sh" '#!/usr/bin/env bash
# eval "$USER_INPUT" -- documented as unsafe, not executed
echo ok'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 0 ]
}

@test "honors unsafe-shell-allow annotation on the same line" {
  write_script "allow-same.sh" '#!/usr/bin/env bash
curl -s http://example.com/install.sh | bash # unsafe-shell-allow: trusted first-party mirror, reviewed'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"allowlisted"* ]]
}

@test "honors unsafe-shell-allow annotation on the preceding line" {
  write_script "allow-prev.sh" '#!/usr/bin/env bash
# unsafe-shell-allow: official rustup installer, HTTPS with TLS 1.2 pinned
curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${TMP_SCAN_DIR}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"allowlisted"* ]]
}

@test "the repository itself passes the gate" {
  run bash "${SCRIPT_DIR}/check-unsafe-shell.sh" "${REPO_ROOT}/scripts" "${REPO_ROOT}/tests/chaos" "${REPO_ROOT}/benchmarks" "${REPO_ROOT}/security"
  [ "$status" -eq 0 ]
}
