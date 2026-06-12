# Changelog

All notable changes to streampay-contracts are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Documentation

- Add developer scripts (`build.sh`, `test.sh`, `fmt-check.sh`, `deny.sh`,
  `clean.sh`) and `docs/scripts.md` reference page.
- Add `docs/local-development.md`, `docs/glossary.md`, `docs/ttl-strategy.md`,
  and `docs/error-codes.md`.
- Expand rustdoc on `StreamInfo` and the persistent-storage helpers in
  `src/stream.rs`.

### Chore

- Add `repository`, `keywords`, and `categories` metadata to `Cargo.toml`.
- Expand `.gitignore` to cover WASM artifacts and common editor files.

## [0.1.0] - 2026-03-24

### Added

- Add StreamPay Soroban contract and tests

### CI

- Add fmt, build, test workflow
- Pin Rust toolchain to 1.93.0
- Add git-cliff configuration for changelog generation
- Add deterministic WASM build Dockerfile
- Add release workflow for tagged WASM builds

### Documentation

- Add README and contributor onboarding
- *(contracts)* Add release process design spec
- *(contracts)* Address spec review findings
- *(contracts)* Add release process implementation plan
- *(contracts)* Add release process guide

### Fixed

- *(contracts)* Move soroban-sdk testutils to dev-dependencies


