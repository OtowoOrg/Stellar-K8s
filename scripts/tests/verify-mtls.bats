#!/usr/bin/env bats
#
# bats suite for scripts/verify-mtls.sh
#
# Covers:
#   - happy path: kubectl shim reports a configured context, all mTLS
#     certificates Ready and all secrets carrying tls.crt -> exit 0
#

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/verify-mtls.sh"

  TEST_TMP="$(mktemp -d)"
  SHIM_BIN="$TEST_TMP/bin"
  mkdir -p "$SHIM_BIN"

  # Canned kubectl: a current-context, Ready certificates, secrets with tls.crt
  cat >"$SHIM_BIN/kubectl" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "config current-context") echo "test-context" ;;
  *"get certificate"*"jsonpath"*) echo "True" ;;
  *"get certificate"*) echo "certificate" ;;
  *"get secret"*"jsonpath"*) echo "Y2VydA==" ;;
  *"get secret"*) echo "secret" ;;
  *) exit 1 ;;
esac
EOF
  chmod +x "$SHIM_BIN/kubectl"

  export ORIGINAL_PATH="$PATH"
  export PATH="$SHIM_BIN:$PATH"
}

teardown() {
  export PATH="$ORIGINAL_PATH"
  rm -rf "$TEST_TMP"
}

@test "verify-mtls.sh succeeds when certificates are Ready and secrets hold tls.crt" {
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" =~ "Verifying mTLS setup in namespace stellar-system" ]]
  [[ "$output" =~ "Ready" ]]
  [[ "$output" =~ "tls.crt present" ]]
  [[ "$output" =~ "mTLS verification passed" ]]
}
