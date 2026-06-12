# Developer Scripts

This page documents the helper scripts under `scripts/`. They wrap the most
common `cargo` and `cargo-deny` invocations so contributors get the same
flags as CI without remembering them.

| Script | Purpose |
|---|---|
| `scripts/build.sh` | Build the release WASM contract artifact. Honors `FEATURES` and `PROFILE` env vars. |
| `scripts/test.sh` | Run the test suite with `--features testutils`. Accepts an optional filter; `VERBOSE=1` enables `--nocapture`. |
| `scripts/fmt-check.sh` | Verify `cargo fmt --all -- --check` passes. Mirrors the CI gate. |
| `scripts/deny.sh` | Install (if needed) and run `cargo-deny` against `deny.toml`. |
| `scripts/clean.sh` | Remove `target/` and `artifacts/`. |
| `scripts/check-wasm-size.sh` | Report optimized WASM size and compare against Soroban limits. |

## Conventions

- All scripts use `set -euo pipefail` so failures stop immediately.
- They are safe to run from any subdirectory because they use `cargo`
  workspace resolution rather than relative paths.
- Output is intentionally minimal so they can be composed in CI pipelines
  without being trimmed by log truncation.

## Adding a new script

1. Place the file under `scripts/` and `chmod +x` it.
2. Top of file: short comment describing purpose, env vars, and usage.
3. Update the table above so contributors can discover it.
