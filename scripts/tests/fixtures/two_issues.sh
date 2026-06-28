#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=../lib/batch.sh
source "$(dirname "$0")/../lib/batch.sh"

create_issue "First" "bug" "body one"
create_issue_with_retry "Second" "bug" "body two"
