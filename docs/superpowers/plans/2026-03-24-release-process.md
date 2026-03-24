# Release Process Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement semver tagging, auto-generated changelog, deterministic WASM builds, and CI release workflow for the StreamPay Soroban contract.

**Architecture:** Pin the Rust toolchain, fix dependency layout, add a Docker-based reproducible WASM builder, configure `git-cliff` for changelog generation, add a GitHub Actions release workflow triggered on `v*` tags, and document the full process in `docs/RELEASE.md`.

**Tech Stack:** Rust, Soroban SDK 22.0, Docker, GitHub Actions, `git-cliff`, `stellar-cli`, `wasm-opt` (Binaryen)

**Spec:** `docs/superpowers/specs/2026-03-24-release-process-design.md`

---

### Task 1: Fix Cargo.toml dependency layout

The `soroban-sdk` testutils feature is currently in `[dependencies]`, which compiles test scaffolding into the release WASM. Move it to `[dev-dependencies]` so the production WASM is clean.

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Update Cargo.toml**

Change `Cargo.toml` to split the SDK dependency:

```toml
[package]
name = "streampay-contracts"
version = "0.1.0"
edition = "2021"
description = "StreamPay Soroban smart contracts for payment streaming"

[lib]
crate-type = ["cdylib", "rlib"]

[features]
default = []
testutils = ["soroban-sdk/testutils"]

[dependencies]
soroban-sdk = "22.0"

[dev-dependencies]
soroban-sdk = { version = "22.0", features = ["testutils"] }
```

- [ ] **Step 2: Verify tests still pass**

Run: `cargo test`
Expected: All 3 tests pass. The `testutils` feature is now only available during `cargo test` (which compiles dev-dependencies).

- [ ] **Step 3: Verify release build compiles**

Run: `cargo build --release`
Expected: Compiles without `testutils` feature in the output binary.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "fix(contracts): move soroban-sdk testutils to dev-dependencies

Prevents test scaffolding from being compiled into the release WASM."
```

---

### Task 2: Pin Rust toolchain

Create `rust-toolchain.toml` as the single source of truth for the compiler version, and update `ci.yml` to use it instead of floating `@stable`.

**Files:**
- Create: `rust-toolchain.toml`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Create rust-toolchain.toml**

Create `rust-toolchain.toml` at the repo root:

```toml
[toolchain]
channel = "1.93.0"
targets = ["wasm32-unknown-unknown"]
components = ["rustfmt"]
```

This pins the exact Rust version and ensures the WASM target is installed automatically.

- [ ] **Step 2: Verify toolchain is picked up**

Run: `rustc --version`
Expected: `rustc 1.93.0` (should match since this is already installed).

- [ ] **Step 3: Update ci.yml to use rust-toolchain.toml**

Replace the `dtolnay/rust-toolchain@stable` step. The full updated `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  fmt-build-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.93.0"
          components: rustfmt
          targets: wasm32-unknown-unknown

      - name: Cache cargo registry and build
        uses: Swatinem/rust-cache@v2

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Build
        run: cargo build

      - name: Run tests
        run: cargo test
```

Key change: `@stable` → `@master` with explicit `toolchain: "1.93.0"`. The `@master` variant of `dtolnay/rust-toolchain` accepts an explicit version. Also adds `wasm32-unknown-unknown` target.

- [ ] **Step 4: Verify tests still pass locally**

Run: `cargo test`
Expected: All 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust-toolchain.toml .github/workflows/ci.yml
git commit -m "ci: pin Rust toolchain to 1.93.0

Adds rust-toolchain.toml as single source of truth for compiler version.
Updates ci.yml to use pinned version instead of floating stable.
Adds wasm32-unknown-unknown target for WASM builds."
```

---

### Task 3: Add git-cliff configuration

Configure `git-cliff` to parse conventional commits and generate a changelog.

**Files:**
- Create: `cliff.toml`

- [ ] **Step 1: Create cliff.toml**

Create `cliff.toml` at the repo root:

```toml
[changelog]
header = """
# Changelog

All notable changes to streampay-contracts are documented here.\n
"""
body = """
{% if version %}\
    ## [{{ version | trim_start_matches(pat="v") }}] - {{ timestamp | date(format="%Y-%m-%d") }}
{% else %}\
    ## [unreleased]
{% endif %}\
{% for group, commits in commits | group_by(attribute="group") %}
    ### {{ group | striptags | trim | upper_first }}
    {% for commit in commits %}
        - {% if commit.scope %}*({{ commit.scope }})* {% endif %}\
            {% if commit.breaking %}[**breaking**] {% endif %}\
            {{ commit.message | upper_first }}\
    {% endfor %}
{% endfor %}\n
"""
footer = ""
trim = true

[git]
conventional_commits = true
filter_unconventional = true
split_commits = false
commit_parsers = [
    { message = "^.*!:", group = "Breaking Changes" },
    { message = "^feat", group = "Added" },
    { message = "^fix", group = "Fixed" },
    { message = "^doc", group = "Documentation" },
    { message = "^refactor", group = "Changed" },
    { message = "^ci", group = "CI" },
    { message = "^release", skip = true },
    { message = "^chore", skip = true },
]
protect_breaking_commits = true
filter_commits = false
tag_pattern = "v[0-9].*"
skip_tags = ""
ignore_tags = ""
topo_order = false
sort_commits = "oldest"
```

- [ ] **Step 2: Install git-cliff locally and verify**

Run: `cargo install git-cliff`
Then: `git-cliff --tag v0.1.0`
Expected: Outputs a changelog with all existing commits grouped by type (feat, docs, ci).

- [ ] **Step 3: Commit**

```bash
git add cliff.toml
git commit -m "ci: add git-cliff configuration for changelog generation

Maps conventional commit prefixes to changelog sections.
Skips release and chore commits."
```

---

### Task 4: Create deterministic WASM build Dockerfile

Build a lightweight Docker image that produces a reproducible WASM artifact with checksum.

**Files:**
- Create: `docker/Dockerfile.build`
- Modify: `.gitignore`

- [ ] **Step 1: Create docker directory**

Run: `mkdir -p docker`

- [ ] **Step 2: Create Dockerfile.build**

Create `docker/Dockerfile.build`:

```dockerfile
ARG RUST_VERSION=1.93.0
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

# Install dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    binaryen \
    && rm -rf /var/lib/apt/lists/*

# Install stellar-cli (pinned version)
ARG STELLAR_CLI_VERSION=22.2.0
RUN cargo install --locked stellar-cli --version ${STELLAR_CLI_VERSION}

# Add WASM target
RUN rustup target add wasm32-unknown-unknown

WORKDIR /build
COPY . .

# Build the optimized WASM
RUN stellar contract build --release \
    && wasm-opt -Oz \
       target/wasm32-unknown-unknown/release/streampay_contracts.wasm \
       -o streampay_contracts.optimized.wasm \
    && sha256sum streampay_contracts.optimized.wasm > streampay_contracts.checksum

# Output stage — copies artifacts out via docker build -o
FROM scratch AS artifacts
COPY --from=builder /build/streampay_contracts.optimized.wasm /
COPY --from=builder /build/streampay_contracts.checksum /
```

The Dockerfile uses `ARG` for both `RUST_VERSION` and `STELLAR_CLI_VERSION` so they can be overridden at build time. The default `RUST_VERSION` should match `rust-toolchain.toml`. When bumping the Rust version in the future, update `rust-toolchain.toml` and rebuild with the new default.

- [ ] **Step 3: Verify stellar-cli version exists**

Run: `cargo search stellar-cli`
Expected: Output shows `stellar-cli` with version `22.2.0` or higher available. If `22.2.0` does not exist, update the `STELLAR_CLI_VERSION` default in the Dockerfile to the latest available version.

- [ ] **Step 4: Add artifacts/ to .gitignore**

Append to `.gitignore`:

```
artifacts/
```

- [ ] **Step 5: Verify Docker build works**

Run: `docker build --platform linux/amd64 -f docker/Dockerfile.build -o artifacts .`
Expected: `artifacts/` directory contains `streampay_contracts.optimized.wasm` and `streampay_contracts.checksum`.

This step may take several minutes on first run (downloading base image, compiling stellar-cli). Subsequent builds use Docker cache.

- [ ] **Step 6: Verify checksum file content**

Run: `cat artifacts/streampay_contracts.checksum`
Expected: A line like `<sha256hash>  streampay_contracts.optimized.wasm`

- [ ] **Step 7: Commit**

```bash
git add docker/Dockerfile.build .gitignore
git commit -m "ci: add deterministic WASM build Dockerfile

Multi-stage Docker build with pinned Rust 1.93.0, stellar-cli, and
wasm-opt. Produces optimized .wasm and .checksum for verification.
Canonical builds target linux/amd64."
```

---

### Task 5: Add CI release workflow

Create the GitHub Actions workflow that triggers on `v*` tag pushes, runs tests, builds the WASM, generates the changelog, and creates a GitHub Release.

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create release.yml**

Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - "v*"

permissions:
  contents: write

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.93.0"
          components: rustfmt
          targets: wasm32-unknown-unknown

      - name: Cache cargo registry and build
        uses: Swatinem/rust-cache@v2

      - name: Validate tag matches Cargo.toml version
        run: |
          TAG_VERSION="${GITHUB_REF_NAME#v}"
          CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
          if [ "$TAG_VERSION" != "$CARGO_VERSION" ]; then
            echo "::error::Tag version ($TAG_VERSION) does not match Cargo.toml version ($CARGO_VERSION)"
            exit 1
          fi

      - name: Run tests
        run: cargo test

      - name: Build deterministic WASM
        run: |
          docker build --platform linux/amd64 \
            -f docker/Dockerfile.build \
            -o artifacts .

      - name: Install git-cliff
        uses: kenji-miyake/setup-git-cliff@v2

      - name: Generate changelog
        id: changelog
        run: |
          TAG_COUNT=$(git tag -l 'v*' | wc -l)
          if [ "$TAG_COUNT" -le 1 ]; then
            git-cliff --tag "$GITHUB_REF_NAME" > RELEASE_NOTES.md
          else
            git-cliff --latest > RELEASE_NOTES.md
          fi

      - name: Create GitHub Release
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          gh release create "$GITHUB_REF_NAME" \
            --title "$GITHUB_REF_NAME" \
            --notes-file RELEASE_NOTES.md \
            artifacts/streampay_contracts.optimized.wasm \
            artifacts/streampay_contracts.checksum
```

Key details:
- `fetch-depth: 0` — full history needed for `git-cliff` and tag validation.
- `permissions: contents: write` — required to create releases.
- Tag count check handles the first-release edge case for `git-cliff`.

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"`
Expected: No errors (valid YAML).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow for tagged WASM builds

Triggered on v* tag push. Validates tag/Cargo.toml match, runs tests,
builds deterministic WASM via Docker, generates changelog with
git-cliff, and creates GitHub Release with artifacts."
```

---

### Task 6: Write RELEASE.md documentation

Create the canonical release guide documenting the full process.

**Files:**
- Create: `docs/RELEASE.md`

- [ ] **Step 1: Create docs/RELEASE.md**

Create `docs/RELEASE.md`:

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add docs/RELEASE.md
git commit -m "docs(contracts): add release process guide

Covers pre-release checklist, semver rules, step-by-step release
instructions, WASM verification, and troubleshooting."
```

---

### Task 7: Update README and generate initial CHANGELOG

Add a Releases section to the README and generate the initial changelog.

**Files:**
- Modify: `README.md`
- Create: `CHANGELOG.md`

- [ ] **Step 1: Add Releases section to README**

Insert the following after the "CI/CD" section (after line 60) in `README.md`:

```markdown
## Releases

Tagged releases follow [semver](https://semver.org/). Each release includes an optimized WASM artifact and SHA-256 checksum.

See [docs/RELEASE.md](docs/RELEASE.md) for the full release process, including how to verify WASM builds.
```

- [ ] **Step 2: Update project structure in README**

Replace the project structure section (lines 62-71 of the original README) to reflect new files. The new content for that section:

~~~markdown
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
~~~

- [ ] **Step 3: Generate initial CHANGELOG.md**

Run: `git-cliff --tag v0.1.0 -o CHANGELOG.md`
Expected: `CHANGELOG.md` created with all existing commits grouped under `## [0.1.0]`.

- [ ] **Step 4: Verify CHANGELOG.md content**

Run: `cat CHANGELOG.md`
Expected: Contains a header, a `## [0.1.0]` section with commits grouped by type (Added, Documentation, CI).

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md
git commit -m "docs(contracts): add releases section and initial changelog

Updates README with release info and project structure.
Generates initial CHANGELOG.md with git-cliff for v0.1.0."
```

---

### Task 8: Final verification

End-to-end validation that everything works together.

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All 3 tests pass.

- [ ] **Step 2: Run format check**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues.

- [ ] **Step 3: Verify Docker WASM build**

Run: `docker build --platform linux/amd64 -f docker/Dockerfile.build -o artifacts .`
Expected: `artifacts/` contains `.wasm` and `.checksum` files.

- [ ] **Step 4: Verify git-cliff generates changelog**

Run: `git-cliff --tag v0.1.0`
Expected: Changelog output with all commits grouped correctly.

- [ ] **Step 5: Verify release workflow YAML is valid**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"`
Expected: No errors.

- [ ] **Step 6: Review all new/modified files**

Run: `git diff main --stat`
Expected: Shows all files from the spec's file table — no extra, no missing.
