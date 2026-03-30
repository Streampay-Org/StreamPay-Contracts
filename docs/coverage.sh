#!/usr/bin/env bash
set -euo pipefail

echo "Running cargo test..."
cargo test

if command -v cargo-tarpaulin >/dev/null 2>&1; then
  echo "Running cargo-tarpaulin for coverage (HTML output)"
  cargo tarpaulin --out Html
  echo "Coverage report: target/tarpaulin-report.html"
else
  echo "cargo-tarpaulin not installed; skip coverage. To install: cargo install cargo-tarpaulin"
fi

echo "Done."
