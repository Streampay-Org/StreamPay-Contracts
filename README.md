# streampay-contracts

Soroban smart contracts for **StreamPay** — continuous payment streaming on the Stellar network.

## Overview

This repo contains the on-chain logic for creating, starting, stopping, and settling payment streams. Contracts are written in Rust using the [Soroban SDK](https://soroban.stellar.org/docs).

### Contract interface

- **`create_stream(payer, recipient, rate_per_second, initial_balance)`** — Create a new stream (payer must auth).
- **`start_stream(stream_id)`** — Start an existing stream.
- **`stop_stream(stream_id)`** — Stop an active stream.
- **`settle_stream(stream_id)`** — Compute and deduct streamed amount since last settlement; returns amount.
- **`get_stream_info(stream_id)`** — Read stream metadata (payer, recipient, rate, balance, timestamps, active).

## Prerequisites

- [Rust](https://rustup.rs/) (stable, with `rustfmt`)
- Optional: [Stellar CLI](https://developers.stellar.org/docs/tools/stellar-cli) for deployment

## Setup for contributors

1. **Clone and enter the repo**
   ```bash
   git clone <repo-url>
   cd streampay-contracts
   ```

2. **Install Rust**
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup component add rustfmt
   ```

3. **Verify setup**
   ```bash
   cargo fmt --all -- --check
   cargo build
   cargo test
   ```

## Scripts

| Command        | Description                |
|----------------|----------------------------|
| `cargo build`  | Build the contract         |
| `cargo test`   | Run unit tests             |
| `cargo fmt`    | Format code                |
| `cargo fmt --all -- --check` | Check formatting (CI) |

## CI/CD

On every push/PR to `main`, GitHub Actions runs:

- Format check: `cargo fmt --all -- --check`
- Build: `cargo build`
- Tests: `cargo test`

Ensure all three pass before merging.

## Releases

Tagged releases follow [semver](https://semver.org/). Each release includes an optimized WASM artifact and SHA-256 checksum.

See [docs/RELEASE.md](docs/RELEASE.md) for the full release process, including how to verify WASM builds.

## Project structure

```
streampay-contracts/
├── src/
│   └── lib.rs                        # Contract and tests
├── docker/
│   └── Dockerfile.build              # Deterministic WASM builder
├── .github/workflows/
│   ├── ci.yml                        # Format, build, test
│   └── release.yml                   # Tagged release workflow
├── docs/
│   └── RELEASE.md                    # Release process guide
├── cliff.toml                        # Changelog generator config
├── rust-toolchain.toml               # Pinned Rust version
├── Cargo.toml
├── CHANGELOG.md
└── README.md
```

## License

MIT
