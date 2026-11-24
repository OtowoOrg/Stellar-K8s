#!/usr/bin/env bash
# Helper script: creates a branch, commits current changes, and pushes to a remote URL.
# Usage: ./scripts/create-argocd-branch.sh [remote-url] [branch-name]
# If `remote-url` is omitted the script pushes to `origin`.

set -euo pipefail

REMOTE_URL="${1:-}"
BRANCH_NAME="${2:-feat/docs/argocd-gitops-guide}"
REMOTE_NAME="push-target"

if [ -n "$REMOTE_URL" ]; then
	# Remove any existing helper remote then add the provided URL under a temporary name
	if git remote | grep -q "^${REMOTE_NAME}$"; then
		git remote remove "$REMOTE_NAME"
	fi
	git remote add "$REMOTE_NAME" "$REMOTE_URL"
	PUSH_REMOTE="$REMOTE_NAME"
else
	PUSH_REMOTE="origin"
fi

git checkout -b "$BRANCH_NAME"
git add docs/examples/ examples/ docs/gitops || git add -A
if git diff --staged --quiet && git diff --quiet; then
	echo "No changes to commit"
else
	git commit -m "docs(argocd): add GitOps guide, interactive generator, and examples"
fi

git push -u "$PUSH_REMOTE" "$BRANCH_NAME"
echo "Created and pushed branch: $BRANCH_NAME to remote: $PUSH_REMOTE"
