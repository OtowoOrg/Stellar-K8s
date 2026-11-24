#!/usr/bin/env bash
# build.sh — compile the hello-world plugin to WebAssembly
#
# Usage:
#   ./build.sh          # release build  (outputs target/wasm32-unknown-unknown/release/hello_world.wasm)
#   ./build.sh --dev    # debug build    (larger binary, faster compile)
#   ./build.sh --opt    # release + wasm-opt size pass (requires wasm-opt on PATH)
#
# See README.md for full prerequisites and step-by-step instructions.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TARGET="wasm32-unknown-unknown"
CRATE_NAME="hello_world"               # Cargo replaces hyphens with underscores
RELEASE_WASM="target/${TARGET}/release/${CRATE_NAME}.wasm"
DEBUG_WASM="target/${TARGET}/debug/${CRATE_NAME}.wasm"

MODE="release"
OPT=false

for arg in "$@"; do
  case "$arg" in
    --dev)  MODE="debug"   ;;
    --opt)  OPT=true       ;;
    --help)
      echo "Usage: $0 [--dev] [--opt]"
      echo "  --dev   Debug build (no size optimisations)"
      echo "  --opt   Run wasm-opt -Oz after release build"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg"
      exit 1
      ;;
  esac
done

# ---------------------------------------------------------------------------
# 1. Ensure the Wasm target is installed
# ---------------------------------------------------------------------------
if ! rustup target list --installed | grep -q "$TARGET"; then
  echo "[build] Adding Rust target $TARGET …"
  rustup target add "$TARGET"
fi

# ---------------------------------------------------------------------------
# 2. Run unit tests on the host (native) before compiling to Wasm
# ---------------------------------------------------------------------------
echo "[build] Running unit tests (native) …"
cargo test --quiet

# ---------------------------------------------------------------------------
# 3. Compile to Wasm
# ---------------------------------------------------------------------------
if [ "$MODE" = "release" ]; then
  echo "[build] Compiling release build …"
  cargo build --target "$TARGET" --release --quiet
  OUTPUT="$RELEASE_WASM"
else
  echo "[build] Compiling debug build …"
  cargo build --target "$TARGET" --quiet
  OUTPUT="$DEBUG_WASM"
fi

SIZE=$(wc -c < "$OUTPUT")
echo "[build] Built: $OUTPUT  (${SIZE} bytes)"

# ---------------------------------------------------------------------------
# 4. Optional: run wasm-opt
# ---------------------------------------------------------------------------
if [ "$OPT" = "true" ]; then
  if ! command -v wasm-opt &>/dev/null; then
    echo "[build] wasm-opt not found — skipping optimisation step."
    echo "        Install with: cargo install wasm-opt  OR  via binaryen package"
  else
    OPT_OUTPUT="${OUTPUT%.wasm}.opt.wasm"
    echo "[build] Running wasm-opt -Oz …"
    wasm-opt -Oz -o "$OPT_OUTPUT" "$OUTPUT"
    OPT_SIZE=$(wc -c < "$OPT_OUTPUT")
    SAVINGS=$(( SIZE - OPT_SIZE ))
    echo "[build] Optimised: $OPT_OUTPUT  (${OPT_SIZE} bytes, saved ${SAVINGS} bytes)"
    OUTPUT="$OPT_OUTPUT"
  fi
fi

# ---------------------------------------------------------------------------
# 5. Print next steps
# ---------------------------------------------------------------------------
echo ""
echo "Build succeeded.  Deploy the plugin:"
echo ""
echo "  kubectl create configmap hello-world-plugin \\"
echo "    --from-file=plugin.wasm=$OUTPUT \\"
echo "    -n stellar-operator-system"
echo ""
echo "Then add the plugin to your operator config (see README.md step 5)."
