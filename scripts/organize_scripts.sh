#!/bin/bash
# Stellar-K8s Scripts Organization
# Moves batch issue creation scripts into a dedicated subfolder.

set -e

echo "📂 Organizing scripts directory..."

mkdir -p scripts/issue_batches

# Move batch scripts
count=0
for f in scripts/create_batch_*_issues.sh; do
    if [ -f "$f" ]; then
        mv "$f" scripts/issue_batches/
        count=$((count + 1))
    fi
done

if [ $count -gt 0 ]; then
    echo "✅ Moved $count batch scripts to scripts/issue_batches/"
else
    echo "ℹ️  No batch scripts found to move."
fi

echo "✨ Scripts organized."
