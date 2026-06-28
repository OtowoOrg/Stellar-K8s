#!/usr/bin/env bats

load "test_helper" 2>/dev/null || true

setup() {
  export REPO="TestOrg/TestRepo"
  export DRY_RUN="false"
  export RETRY_MAX_ATTEMPTS=2
  export RETRY_DELAY_SECONDS=0

  # Source the common helper script
  source "${BATS_TEST_DIRNAME}/../lib/common.sh"
  
  # Mock gh command
  gh() {
    if [[ "$1" == "issue" && "$2" == "create" ]]; then
      if [[ "$GH_MOCK_SHOULD_FAIL" == "true" ]]; then
        return 1
      fi
      return 0
    fi
    return 1
  }
  export -f gh
}

@test "dry_run mode prints and does not fail" {
  export DRY_RUN="true"
  run create_issue "Test Title" "bug" "Test Body"
  
  [ "$status" -eq 0 ]
  [[ "$output" == *"[DRY RUN] title:  Test Title"* ]]
}

@test "create_issue succeeds on first try" {
  export GH_MOCK_SHOULD_FAIL="false"
  run create_issue "Test Title" "bug" "Test Body"
  
  [ "$status" -eq 0 ]
  [[ "$output" == *"✓ Issue created: Test Title"* ]]
}

@test "create_issue fails after max attempts" {
  export GH_MOCK_SHOULD_FAIL="true"
  export RETRY_MAX_ATTEMPTS=2
  run create_issue "Test Title" "bug" "Test Body"
  
  [ "$status" -eq 1 ]
  [[ "$output" == *"ERROR: Failed to create issue after 2 attempts: Test Title"* ]]
}

@test "REPO validation fails on invalid format" {
  export REPO="Invalid/Repo/Format"
  run bash -c "source ${BATS_TEST_DIRNAME}/../lib/common.sh"
  
  [ "$status" -eq 1 ]
  [[ "$output" == *"ERROR [validate repo]: REPO='Invalid/Repo/Format' is not a valid 'owner/name' format."* ]]
}
