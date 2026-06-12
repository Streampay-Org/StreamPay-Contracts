# Testing Strategy

This page captures the conventions the project uses for writing and
maintaining tests. New contributors should read it before adding tests so
the suite stays cohesive.

## Layers

1. **Unit tests** (inline in `src/lib.rs` under `#[cfg(test)] mod tests`).
   Cover individual entry points with a fresh `Env::default()` per test.
2. **Snapshot tests** (`test_snapshots/test/*.json`). Soroban writes one
   JSON per `#[test]` capturing the ledger state at the end of the test.
   They guard against unintended storage-layout changes.
3. **Future**: an integration suite against a local stellar quickstart
   network, gated behind the `integration` cargo feature.

## Conventions

- Name tests after the behavior they assert, not the function under test.
  `test_settle_inactive_returns_zero` over `test_settle_stream_5`.
- One assertion per test where possible. Combine related assertions only
  if they share setup that would otherwise be duplicated verbatim.
- Use `MockAuthInvoke` for authorization tests; never rely on the default
  permissive mode for production-path assertions.
- Time-based tests use `env.ledger().set(LedgerInfo { ... })` rather than
  real sleeps.

## Snapshot maintenance

Snapshot JSON files in `test_snapshots/test/` are committed. When a
storage-layout change is intentional:

1. Run `UPDATE_SNAPSHOTS=1 cargo test` to refresh them.
2. Review the diff against the previous snapshot in your PR.
3. Note the schema-version bump in `docs/schema-versioning.md` and the
   `CHANGELOG.md`.

If the diff is unexpected, treat it as a regression and investigate
before refreshing.

## Coverage target

For PRs that touch contract logic, target **95% line coverage** for the
code you changed. The CI does not enforce this yet but reviewers will
ask for justification when it is missed.
