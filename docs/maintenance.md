# Maintenance Playbook

Routine maintenance tasks for `streampay-contracts` so the project does not
go stale between feature work.

## Weekly

- Run `./scripts/deny.sh` and address any new advisory hits.
- Confirm `cargo update --dry-run` does not produce a backwards-incompatible
  bump of `soroban-sdk`. If it does, open a tracking issue.

## Monthly

- Touch long-lived demo streams with a no-op `settle_stream` so their
  persistent storage TTL stays above the threshold.
- Verify the deployment artifact hash for the latest tag against the
  on-chain contract instance with `./scripts/wasm-hash.sh`.

## Quarterly

- Walk through the documents under `docs/` and prune anything that has
  diverged from current behavior.
- Re-read `SECURITY.md` and update SLAs if maintainer availability has
  changed.

## On every release

1. Bump `version` in `Cargo.toml` and `VERSION` in `src/lib.rs`.
2. Move `Unreleased` notes in `CHANGELOG.md` into a new dated section.
3. Tag `vX.Y.Z` and let the release workflow publish the WASM.
4. Announce in the relevant developer channel along with the WASM hash.

## On a paged incident

- Treat any user report of stuck balances as a P1 until proved otherwise.
- Capture the affected stream ids and ledger range before doing anything
  else; they are the only way to reconstruct events for archived streams.
- File a post-mortem within 5 business days and link it from the
  incident's GitHub issue.
