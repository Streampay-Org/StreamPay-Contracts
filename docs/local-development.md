# Local Development

A short walkthrough for getting a working contributor environment for
`streampay-contracts`.

## Prerequisites

- Rust toolchain pinned in `rust-toolchain.toml`. `rustup` will install the
  pinned version automatically the first time you run `cargo`.
- `rustfmt` component: `rustup component add rustfmt`.
- (Optional) `stellar-cli` for deploying to Futurenet/Testnet.
- (Optional) Docker, if you want the deterministic WASM builder in
  `docker/Dockerfile.build`.

## First build

```bash
git clone <repo-url>
cd StreamPay-Contracts
cargo build
cargo test
```

The first build downloads the Soroban SDK and its transitive dependencies; it
typically takes a few minutes on a fresh machine.

## Useful commands

| Command | What it does |
|---|---|
| `cargo build` | Native build, fastest feedback loop. |
| `./scripts/build.sh` | Release build that produces the deployable WASM. |
| `./scripts/test.sh` | Test suite with `testutils` feature enabled. |
| `./scripts/fmt-check.sh` | Mirrors the CI rustfmt gate. |
| `./scripts/check-wasm-size.sh` | Checks the WASM size against Soroban's 256KB limit. |

## Editor setup

Any editor with `rust-analyzer` support works. If you use VS Code, add the
following to your workspace settings to match the project's style:

```json
{
  "rust-analyzer.cargo.features": ["testutils"],
  "editor.formatOnSave": true
}
```

## Troubleshooting

- **"error: linking with `cc` failed"** when building for WASM: install the
  `wasm32-unknown-unknown` target with `rustup target add wasm32-unknown-unknown`.
- **Tests panic with "stream not found"**: the test snapshot likely got
  out of sync with code changes; re-run with `UPDATE_SNAPSHOTS=1` to refresh.
