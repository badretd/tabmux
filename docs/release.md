# Release guide

> **Status:** Phase 12. This document is the independent checklist for cutting,
> verifying, and publishing a `tabmux` release.

## 1. Release model

`tabmux` uses semantic versioning and tags releases as `vMAJOR.MINOR.PATCH`
(for example `v0.1.0`). The package version in `Cargo.toml` must match the tag
without the leading `v`.

The repository is set up for two release paths:

1. **Automated (recommended):** push a matching `v*` tag. The
   `.github/workflows/release.yml` workflow runs checks, builds platform
   archives, generates an SBOM and checksums, and publishes a GitHub release.
2. **Independent/manual:** run `scripts/build-release.sh`, verify the
   artifacts locally, and publish them yourself. This path does not depend on
   GitHub Actions.

Both paths use the same binary, smoke suite, release notes, license files, and
SBOM generator.

## 2. Preconditions

Install:

- Rust 1.88 or newer with `rustfmt` and `clippy`;
- Python 3.9 or newer (fixture scripts and the SBOM generator);
- a POSIX shell, `tar`, and either `sha256sum` or `shasum`;
- `git`.

The following must be true before tagging:

- [ ] `Cargo.toml` `version` is the intended release version.
- [ ] `Cargo.lock` is committed and up to date (`cargo build --locked` works).
- [ ] `README.md`, `RELEASE_NOTES.md`, `docs/devplan.md`, `docs/features.md`,
      and `docs/implementation-notes.md` describe the shipped behavior.
- [ ] `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE` are present.
- [ ] The working tree is clean (`git status --short` is empty).
- [ ] The `docs/release-notes` content lists scope, non-goals, and known
      fidelity limits.

## 3. Local verification

Run the full quality gate:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --all-features
cargo test --all-targets --no-default-features --features core,excel
./scripts/smoke.sh
TABMUX_FEATURES=sql-duckdb ./scripts/smoke.sh
```

Optional dependency and performance gates:

```sh
cargo deny check
cargo audit
./scripts/perf-smoke.sh
```

The fuzz targets are nightly-only and non-blocking; see `fuzz/README.md`.

### Version string

```sh
cargo run -- --version
```

Expected shape:

```text
tabmux 0.1.0 (commit 72fa4a0; features core,excel,html,rust_xlsxwriter)
```

The commit is embedded by `build.rs`. It is `unknown` outside a git checkout.
The feature list is the enabled Cargo feature set.

## 4. Build the release artifact

```sh
./scripts/build-release.sh
```

This script:

1. checks formatting, clippy, and the all-feature test suite (skip with
   `TABMUX_SKIP_CHECKS=1` if already run);
2. builds `target/release/tabmux` with `--locked`;
3. enforces a binary-size budget (50 MiB by default, override with
   `TABMUX_MAX_BINARY_MB`);
4. runs the full smoke suite against the release binary (skip with
   `TABMUX_SKIP_SMOKE=1`);
5. packages the binary, README, license files, release notes, and docs;
6. generates a CycloneDX 1.5 SBOM;
7. creates a `.tar.gz` archive and `SHA256SUMS`.

Artifacts are written to `dist/`:

```text
dist/
  tabmux-0.1.0-<target>.tar.gz
  SBOM-0.1.0.cdx.json
  SHA256SUMS
  tabmux-0.1.0-<target>/
```

## 5. Verify the artifacts

Use a clean shell and verify before publishing:

```sh
cd dist
sha256sum -c SHA256SUMS          # macOS: shasum -a 256 -c SHA256SUMS
tar -tzf tabmux-0.1.0-<target>.tar.gz
```

Extract into a temporary directory and run:

```sh
mkdir /tmp/tabmux-check && tar -xzf tabmux-0.1.0-<target>.tar.gz -C /tmp/tabmux-check
/tmp/tabmux-check/tabmux-0.1.0-<target>/tabmux --version
/tmp/tabmux-check/tabmux-0.1.0-<target>/tabmux formats
```

Run the smoke suite from the extracted directory if desired:

```sh
cd /tmp/tabmux-check/tabmux-0.1.0-<target>
./tabmux --version
```

`scripts/smoke.sh` expects the repository fixtures, so it is normally run from
the source checkout against the release binary:

```sh
./scripts/smoke.sh target/release/tabmux
```

Inspect the SBOM:

```sh
python3 -m json.tool SBOM-0.1.0.cdx.json >/dev/null
grep -c '"type": "library"' SBOM-0.1.0.cdx.json
```

The SBOM must list every package in `Cargo.lock` except `tabmux` itself.

## 6. Tag and publish

Once the artifacts and checksums pass:

```sh
git tag -a v0.1.0 -m "tabmux v0.1.0"
git push origin v0.1.0
```

The tag triggers the release workflow. If publishing manually instead:

1. Create a release on the hosting service for the tag.
2. Attach every file from `dist/`:
   - the platform archive(s);
   - `SBOM-0.1.0.cdx.json`;
   - `SHA256SUMS`.
3. Use `RELEASE_NOTES.md` as the release body (or the hosting service's
   generated notes plus the same scope/limits section).
4. Do not attach `target/` or debug artifacts.

### Automated workflow expectations

`.github/workflows/release.yml` verifies that `v<Cargo version>` matches the
pushed tag, runs the quality gate, builds release archives, generates the SBOM
and checksums, and uploads them to the GitHub release. The workflow requires
`contents: write` permission for the release step.

## 7. Independent verification by a third party

An independent reviewer can verify a release without trusting the build host:

1. Download the archive and `SHA256SUMS` from the release page.
2. Recompute the archive checksum and compare it with the published value.
3. Extract the archive and run `--version` and `formats`.
4. Compare the embedded commit in `--version` with the tag's commit.
5. Check `SBOM-0.1.0.cdx.json` against `Cargo.lock` from the matching source
   tag.
6. Rebuild from the exact source tag and compare behavior:
   ```sh
   git checkout v0.1.0
   cargo build --release --locked
   ./scripts/smoke.sh target/release/tabmux
   ```
   Byte-for-byte reproducibility is not promised for all platforms because of
   compiler and linker differences, but data outputs, CLI behavior, and the
   version string should match.
7. Review `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE`.

## 8. If something is wrong

- Do **not** move or reuse a published tag. Delete the bad release and tag, fix
  the issue, bump the patch version, and publish a new tag.
- If a security advisory affects a dependency, update `Cargo.lock`, rerun
  `cargo audit`, regenerate the SBOM, and publish a patch release.
- Regenerate fixtures deterministically with the scripts in `scripts/` if a
  fixture changes. Verify the diff before committing.

## 9. License policy

`tabmux` is dual-licensed under **MIT OR Apache-2.0**. Keep both license files
and `NOTICE` in every binary archive. New dependencies must be compatible with
that policy; `deny.toml` is the automated check.
