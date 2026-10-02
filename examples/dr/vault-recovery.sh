#!/usr/bin/env bash

# Vault Recovery Script for Stellar Validator
# -------------------------------------------------
# This script fetches the validator seed keys from HashiCorp Vault,
# creates a Kubernetes Secret manifest, and applies it to the cluster.
# It assumes the Vault token or IAM role is already available in the
# environment (via $VAULT_TOKEN or the default auth method).
# -------------------------------------------------
set -euo pipefail

# Configuration – customize these values for your environment
VAULT_ADDR="${VAULT_ADDR:-https://vault.example.com}"   # Vault address
VAULT_SECRET_PATH="${VAULT_SECRET_PATH:-secret/data/stellar/validator}"  # Path to the secret containing the seed
K8S_NAMESPACE="${K8S_NAMESPACE:-stellar}"   # Namespace where the secret will be created
SECRET_NAME="${SECRET_NAME:-stellar-validator-keys}"   # Name of the Kubernetes Secret

# Retrieve the seed from Vault (expects a JSON field "seed")
echo "Fetching validator seed from Vault..."
SEED=$(vault kv get -field=seed "$VAULT_SECRET_PATH")

# Encode the seed in base64 for the secret manifest
SEED_B64=$(printf "%s" "$SEED" | base64)

# Create a temporary manifest file
TMPFILE=$(mktemp /tmp/stellar-validator-secret.XXXXXX.yaml)
cat > "$TMPFILE" <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: $SECRET_NAME
  namespace: $K8S_NAMESPACE
type: Opaque
stringData:
  seed: "$SEED"
EOF

# Apply the secret to the cluster
echo "Applying Kubernetes Secret..."
kubectl apply -f "$TMPFILE"

# Clean up the temporary file
rm -f "$TMPFILE"

echo "Vault recovery complete. Secret \"$SECRET_NAME\" created in namespace \"$K8S_NAMESPACE\"."
