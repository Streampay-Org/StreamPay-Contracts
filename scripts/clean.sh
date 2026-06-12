#!/usr/bin/env bash
# Remove build artifacts produced by cargo and wasm builds.

set -euo pipefail

echo "Cleaning cargo target/ ..."
cargo clean

if [ -d artifacts ]; then
    echo "Removing artifacts/ ..."
    rm -rf artifacts
fi

echo "Done."
