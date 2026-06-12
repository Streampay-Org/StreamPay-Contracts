#!/usr/bin/env bash
# Verify Rust source is rustfmt-clean. Mirrors the check used in CI.
#
# Exits non-zero if any file would be reformatted.

set -euo pipefail

echo "Checking rustfmt compliance..."
cargo fmt --all -- --check
echo "rustfmt: clean"
