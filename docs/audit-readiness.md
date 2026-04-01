# Audit Readiness Checklist

Purpose
-------
This document is a compact checklist to prepare the `streampay-contracts` crate for a security audit. It lists how to run tests and coverage, the key files to review, a short threat-model / limitations summary, and a small set of reproducible commands for auditors.

Quick commands
--------------
- Run unit tests:

  ```bash
  cargo test
  ```

- Recommended coverage (optional): install `cargo-tarpaulin` and run

  ```bash
  cargo install cargo-tarpaulin || true
  cargo tarpaulin --out Html
  # open coverage report in target/tarpaulin-report.html
  ```

Notes: coverage tooling for Rust/WASM varies by environment; auditors may prefer `grcov` or CI-integrated coverage. The source includes unit tests driven by `soroban-sdk` testutils (hosted test environment), which `cargo test` runs locally.

Key files to review
-------------------
- `src/lib.rs` — contract entrypoints, validation logic, and tests.
- `Cargo.toml` — dependency and dev-dependency surface (check SDK/testutils versions).
- `README.md` and `docs/*` — usage and design rationale.

Threat model and security notes (brief)
-------------------------------------
- The contract is a Soroban smart contract handling streaming payments. Primary risks:
  - Incorrect accounting of accrued amounts (rounding, overflow, saturation)
  - Authorization errors (payer / recipient actions)
  - Storage/TTL misuse leading to accidental archive of active streams
- Mitigations in this codebase:
  - Use of saturating math (`saturating_mul`, `saturating_sub`) to avoid overflow and ensure safe capping.
  - `require_auth()` checks on payer-restricted operations.
  - Archive requires zero balance and inactive state.

Known limitations / scope decisions
---------------------------------
- Minimum rate and minimum initial balance are compile-time constants (`MIN_RATE_PER_SECOND`, `MIN_INITIAL_BALANCE`) in `src/lib.rs`. They are enforced on `create_stream`. If deploy-time tunability is required, consider storing minima in instance storage with admin-only setters.
- Coverage: Soroban/WASM instrumentation and coverage measurement can be environment-specific; achieving line-level coverage for generated WASM may require CI setup.

Tests and current status
------------------------
- Unit tests exercise create/start/stop/settle/archive flows and the new view-accrual logic. Example assertions validate behaviour at boundaries (minima, capping by balance, inactive streams).
- Current `cargo test` run (local): 18 passed; 0 failed.

Recommended reviewer checklist
-----------------------------
1. Review `src/lib.rs` for correctness of arithmetic and auth checks.
2. Verify tests in `src/lib.rs` cover boundary cases (minima and accrual capping).
3. Check `Cargo.toml` for pinned versions of `soroban-sdk` and testutils.
4. Suggest any additional attacker scenarios (reentrancy-like concerns, though Soroban model differs from EVM).

Contact
-------
If you need help reproducing coverage reports in CI or want a small CI job that runs `cargo tarpaulin`, open an issue or contact the maintainers.
