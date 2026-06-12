#!/usr/bin/env bash
# Build the StreamPay Soroban contract WASM artifact.
#
# Outputs the optimized WASM under target/wasm32-unknown-unknown/release/.
# Set FEATURES to enable cargo features (default: none).
#
# Usage:
#   ./scripts/build.sh                # release build
#   FEATURES=testutils ./scripts/build.sh

set -euo pipefail

FEATURES="${FEATURES:-}"
PROFILE="${PROFILE:-release}"
TARGET="wasm32-unknown-unknown"

echo "Building streampay-contracts (profile=$PROFILE, features=${FEATURES:-<none>})"

if [ -n "$FEATURES" ]; then
    cargo build --profile "$PROFILE" --target "$TARGET" --features "$FEATURES"
else
    cargo build --profile "$PROFILE" --target "$TARGET"
fi

WASM="target/$TARGET/$PROFILE/streampay_contracts.wasm"
if [ -f "$WASM" ]; then
    echo "Built: $WASM"
    SIZE=$(wc -c <"$WASM" | tr -d ' ')
    echo "Size:  $SIZE bytes"
else
    echo "WARN: expected artifact not found at $WASM" >&2
fi
