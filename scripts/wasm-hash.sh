#!/usr/bin/env bash
# Print the SHA-256 of the release WASM artifact.
#
# Useful for cross-checking what a remote node deployed against what was
# built locally. Run `./scripts/build.sh` first.

set -euo pipefail

WASM="target/wasm32-unknown-unknown/release/streampay_contracts.wasm"

if [ ! -f "$WASM" ]; then
    echo "WASM not found at $WASM" >&2
    echo "Run ./scripts/build.sh first." >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$WASM"
else
    shasum -a 256 "$WASM"
fi
