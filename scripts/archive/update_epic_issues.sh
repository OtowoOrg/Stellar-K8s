#!/usr/bin/env bash
set -euo pipefail

REPO="${REPO:-OtowoOrg/Stellar-K8s}"

echo "Updating epic issues to remove 'Estimated Effort' sections..."

# Get all epic issues
ISSUE_NUMBERS=(871 870 869 868 867 866 865 864 863 862 861 860)

for issue_num in "${ISSUE_NUMBERS[@]}"; do
  echo "Processing issue #$issue_num..."
  
  # Get current issue body
  body=$(gh issue view "$issue_num" --repo "$REPO" --json body -q .body)
  
  # Remove the "Estimated Effort" section using sed
  # This removes from "## Estimated Effort" to the line before "## References"
  updated_body=$(echo "$body" | sed '/## Estimated Effort/,/## References/{/## Estimated Effort/d; /## References/!d;}')
  
  # Update the issue
  echo "$updated_body" | gh issue edit "$issue_num" --repo "$REPO" --body-file -
  
  echo "✓ Updated issue #$issue_num"
done

echo "✅ All epic issues updated successfully!"
