# Release Process

Guide for cutting releases of streampay-contracts.

## Pre-release Checklist

- [ ] All tests pass (`cargo test`)
- [ ] Version bumped in `Cargo.toml`
- [ ] `soroban-sdk` testutils feature is in `[dev-dependencies]`, not `[dependencies]`
- [ ] Changes are merged to `main`

## Semver Bump Rules

| Bump | When |
|------|------|
| **Major** | Breaking entry point changes (renamed/removed functions, changed params) OR storage-migration-required changes |
| **Minor** | New entry points, new features backward-compatible with existing streams |
| **Patch** | Bug fixes, doc changes, or any non-breaking change that alters the compiled WASM hash |

Every on-chain-distinguishable build (different WASM hash) gets at minimum a patch bump.

## Cutting a Release

1. **Bump version** in `Cargo.toml`:
   ```toml
   version = "X.Y.Z"
   ```

2. **Commit the version bump**:
   ```bash
   git add Cargo.toml
   git commit -m "release(contracts): vX.Y.Z"
   ```

3. **Tag the release**:
   ```bash
   git tag vX.Y.Z
   ```

4. **Push to main with tag**:
   ```bash
   git push origin main --tags
   ```

5. **CI takes over** — the release workflow:
   - Validates the tag matches `Cargo.toml`
   - Runs tests
   - Builds a deterministic WASM via Docker
   - Generates a changelog entry with `git-cliff`
   - Creates a GitHub Release with the `.wasm` and `.checksum` attached

## Verifying a WASM Build

Anyone can verify that a release artifact is reproducible:

```bash
# Clone the repo at the release tag
git checkout vX.Y.Z

# Build with Docker (use --platform on Apple Silicon)
docker build --platform linux/amd64 -f docker/Dockerfile.build -o artifacts .

# Compare checksums
cat artifacts/streampay_contracts.checksum
# Should match the .checksum file from the GitHub Release
```

The canonical hash is produced on `linux/amd64`. On Apple Silicon, the `--platform linux/amd64` flag is required for matching hashes.

## Version Sources

These must stay in sync on every release:

- `Cargo.toml` `version` field
- Git tag (`vX.Y.Z`)

## Troubleshooting

### Tag version doesn't match Cargo.toml
The release workflow validates that the git tag matches the version in `Cargo.toml`. If they diverge, the workflow fails. Fix by deleting the incorrect tag and re-tagging:

```bash
git tag -d vX.Y.Z
git push origin :refs/tags/vX.Y.Z
# Fix Cargo.toml, commit, then re-tag
git tag vX.Y.Z
git push origin main --tags
```

### Docker build fails
- Ensure Docker is installed and running
- Check that `docker/Dockerfile.build` references valid pinned versions
- Try `docker build --no-cache` to rebuild from scratch
