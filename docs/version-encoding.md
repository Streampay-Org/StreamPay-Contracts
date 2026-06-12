# Version Encoding

`streampay-contracts` exposes a single `u32` constant called `VERSION`
through the `version()` entry point. This page documents the packing
scheme and the operational rules around bumping it.

## Packing

```
VERSION = major * 1_000_000 + minor * 1_000 + patch
```

| Semver | u32 |
|---|---|
| `0.1.0` | `1_000` |
| `0.2.0` | `2_000` |
| `1.0.0` | `1_000_000` |
| `1.2.3` | `1_002_003` |

The scheme caps each component at `999`. That is plenty for foreseeable
versions; if you ever need more, treat it as a major release that also
publishes a new contract.

## Decoding

```rust
fn decode(v: u32) -> (u32, u32, u32) {
    let major = v / 1_000_000;
    let minor = (v / 1_000) % 1_000;
    let patch = v % 1_000;
    (major, minor, patch)
}
```

## Release checklist

When cutting a release:

1. Bump `version` in `Cargo.toml`.
2. Bump the `VERSION` constant in `src/lib.rs` to the matching packed value.
3. Update `CHANGELOG.md` (move `Unreleased` into the new dated section).
4. Tag the commit `vX.Y.Z`. The release workflow builds the WASM and
   publishes a SHA-256 checksum.

A test (`test_version_matches_const`) enforces that the runtime value of
`version()` agrees with the Cargo manifest, so forgetting step 2 is
caught in CI rather than at deploy time.
