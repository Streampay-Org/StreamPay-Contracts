#!/usr/bin/env bash
# Run the StreamPay Soroban contract test suite.
#
# Always runs with the `testutils` feature so that `soroban-sdk` test helpers
# (`Env::default`, mocked auth, ledger snapshots) are available.
#
# Usage:
#   ./scripts/test.sh                # run all tests
#   ./scripts/test.sh test_create    # filter by name
#   VERBOSE=1 ./scripts/test.sh      # pass --nocapture

set -euo pipefail

FILTER="${1:-}"
EXTRA_ARGS=()
if [ "${VERBOSE:-0}" = "1" ]; then
    EXTRA_ARGS+=(--nocapture)
fi

echo "Running streampay-contracts tests"
if [ -n "$FILTER" ]; then
    cargo test --features testutils -- "$FILTER" "${EXTRA_ARGS[@]}"
else
    cargo test --features testutils -- "${EXTRA_ARGS[@]}"
fi
