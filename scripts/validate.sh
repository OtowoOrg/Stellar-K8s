#!/bin/bash
set -e
echo "Starting local validation..."
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --no-run # Just check if it compiles
echo "Validation complete."
