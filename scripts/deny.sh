#!/usr/bin/env bash
# Run cargo-deny against the project's deny.toml policy.
#
# Installs cargo-deny on demand if it is not already on PATH.

set -euo pipefail

if ! command -v cargo-deny >/dev/null 2>&1; then
    echo "cargo-deny not found, installing..."
    cargo install cargo-deny --locked
fi

echo "Running cargo deny check"
cargo deny check
