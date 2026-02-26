#!/bin/bash
set -e

echo "Building image-registry-validator plugin..."

# Build the Wasm binary
cargo build --target wasm32-unknown-unknown --release

WASM_FILE="target/wasm32-unknown-unknown/release/image_registry_validator.wasm"

if [ -f "$WASM_FILE" ]; then
    SIZE=$(wc -c < "$WASM_FILE")
    echo "✓ Built successfully: $WASM_FILE ($SIZE bytes)"
    
    # Optimize if wasm-opt is available
    if command -v wasm-opt &> /dev/null; then
        echo "Optimizing with wasm-opt..."
        wasm-opt -Oz -o "${WASM_FILE%.wasm}_opt.wasm" "$WASM_FILE"
        OPT_SIZE=$(wc -c < "${WASM_FILE%.wasm}_opt.wasm")
        echo "✓ Optimized: ${WASM_FILE%.wasm}_opt.wasm ($OPT_SIZE bytes)"
        SAVINGS=$((SIZE - OPT_SIZE))
        echo "  Saved: $SAVINGS bytes ($(( SAVINGS * 100 / SIZE ))%)"
    else
        echo "ℹ wasm-opt not found, skipping optimization"
        echo "  Install with: cargo install wasm-opt"
    fi
else
    echo "✗ Build failed: $WASM_FILE not found"
    exit 1
fi
