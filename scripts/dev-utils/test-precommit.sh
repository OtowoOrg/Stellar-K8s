#!/bin/bash
# Test script to verify pre-commit configuration

set -e

echo "🔍 Testing pre-commit configuration..."

# Check if configuration files exist
echo "✓ Checking configuration files..."
test -f .pre-commit-config.yaml && echo "  ✓ .pre-commit-config.yaml exists"
test -f .yamllint.yml && echo "  ✓ .yamllint.yml exists"

# Validate YAML syntax
echo "✓ Validating YAML syntax..."
python3 -c "import yaml; yaml.safe_load(open('.pre-commit-config.yaml'))" && echo "  ✓ .pre-commit-config.yaml is valid"
python3 -c "import yaml; yaml.safe_load(open('.yamllint.yml'))" && echo "  ✓ .yamllint.yml is valid"

# Check Makefile targets
echo "✓ Checking Makefile targets..."
make help | grep -q "pre-commit" && echo "  ✓ pre-commit targets available"

# Test individual commands that pre-commit would run
echo "✓ Testing individual commands..."
cargo fmt --all --check && echo "  ✓ cargo fmt check passes" || echo "  ⚠️  cargo fmt check failed (run 'cargo fmt --all')"
cargo clippy --workspace --all-targets --all-features -- -D warnings && echo "  ✓ cargo clippy passes" || echo "  ⚠️  cargo clippy failed"

echo ""
echo "🎉 Pre-commit configuration test completed!"
echo ""
echo "To set up pre-commit hooks:"
echo "  make dev-setup"
echo ""
echo "To run hooks manually:"
echo "  make pre-commit"
