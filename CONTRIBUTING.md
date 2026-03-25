# Contributing to StreamPay Contracts

Thanks for taking the time to contribute!

This repository contains the `streampay-contracts` Soroban smart contract crate.

## Quick start

## Workflow

1. Fork the repository (or create a branch if you have write access).
2. Create a branch following the naming rules below.
3. Make focused changes with tests.
4. Push your branch and open a PR against `main`.

### Prerequisites

- Rust (stable)
- `rustfmt` (`rustup component add rustfmt`)
- Optional: Stellar CLI for deployment

### Soroban SDK version

This repo pins `soroban-sdk` to **`22.0`** (see `Cargo.toml`). Please keep PRs compatible with that version unless the PR is explicitly upgrading the SDK.

### Build, format, test (required)

Run these locally before opening a PR:

```bash
cargo fmt --all -- --check
cargo build
cargo test
```

If you touch contract logic, add/extend tests for edge cases and include a short note in the PR description about what you validated.

## Branch naming

Use short, descriptive branch names in this format:

`<type>/<kebab-case-summary>`

Recommended `type` values:

- `feat/` — new functionality
- `fix/` — bug fixes
- `docs/` — documentation-only changes
- `chore/` — tooling / maintenance
- `refactor/` — refactors without behavior changes
- `test/` — tests-only changes

Example:

```bash
git checkout -b docs/contributing-contracts
```

## Commit messages

Prefer Conventional Commits:

`type(scope): short summary`

Example:

`docs(contracts): CONTRIBUTING guide for contract developers`

## Code style

- Keep changes small and reviewable.
- Match the existing Rust style and pass `cargo fmt`.
- Avoid breaking the public contract interface without an explicit versioning plan.
- Prefer clear, deterministic contract behavior (auth checks, storage keys, arithmetic).

## Pull request expectations

When opening a PR:

- Describe **what** changed and **why** (link the issue if applicable).
- Include the commands you ran and their results (at minimum: `cargo test`).
- Add short **security notes** for contract-logic changes (e.g., auth, overflow/underflow, invariants).
- Be responsive to review feedback and iterate quickly.

## Timelines

If you’re assigned an issue, aim to open a draft PR or progress update within **96 hours** so others know it’s actively being worked on.

## Security

If you believe you’ve found a vulnerability, please **do not** open a public issue. See `SECURITY.md`.

## Coverage guideline

For PRs that touch contract logic, target **minimum 95% test coverage** for the code you changed (or explicitly justify any gaps in the PR description).
