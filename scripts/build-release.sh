#!/usr/bin/env bash
#
# Build and package a release artifact for the current platform.
#
# Usage:
#   ./scripts/build-release.sh
#
# Environment:
#   TABMUX_SKIP_CHECKS=1   skip fmt/clippy/test (used by the release workflow,
#                          which has already run them in a separate job)
#   TABMUX_SKIP_SMOKE=1    skip the release-binary smoke test
#   TABMUX_MAX_BINARY_MB=50 binary size budget
#
# Outputs (under dist/):
#   tabmux-<version>-<target>.tar.gz
#   SBOM-<version>.cdx.json
#   SHA256SUMS
#
# The script is intentionally portable: it works with GNU tar/sha256sum on
# Linux/macOS and with the tools available in Git Bash on Windows.

set -euo pipefail

cd "$(dirname "$0")/.."

SKIP_CHECKS="${TABMUX_SKIP_CHECKS:-0}"
SKIP_SMOKE="${TABMUX_SKIP_SMOKE:-0}"
MAX_BINARY_MB="${TABMUX_MAX_BINARY_MB:-50}"

version="$(python3 - <<'PY'
import re
text = open("Cargo.toml", encoding="utf-8").read()
match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
if not match:
    raise SystemExit("missing package version in Cargo.toml")
print(match.group(1))
PY
)"

target="$(rustc -vV | awk '/^host:/ { print $2 }')"
[[ -n "$version" && -n "$target" ]] || {
  echo "build-release: could not determine version or target" >&2
  exit 1
}

echo "build-release: tabmux $version for $target"

if [[ "$SKIP_CHECKS" != "1" ]]; then
  echo "build-release: checking formatting"
  cargo fmt --all --check
  echo "build-release: running clippy"
  cargo clippy --all-targets --all-features -- -D warnings
  echo "build-release: running tests"
  cargo test --all-targets --all-features
fi

echo "build-release: building"
cargo build --release --locked

binary="target/release/tabmux"
[[ -f "$binary.exe" ]] && binary="$binary.exe"
[[ -x "$binary" || -f "$binary" ]] || {
  echo "build-release: release binary not found: $binary" >&2
  exit 1
}

size_bytes="$(wc -c <"$binary" | tr -d ' ')"
size_mb=$((size_bytes / 1024 / 1024))
echo "build-release: binary size ${size_mb} MiB (${size_bytes} bytes)"
if (( size_mb > MAX_BINARY_MB )); then
  echo "build-release: binary exceeds ${MAX_BINARY_MB} MiB budget" >&2
  exit 1
fi

if [[ "$SKIP_SMOKE" != "1" ]]; then
  echo "build-release: running smoke test against the release binary"
  ./scripts/smoke.sh "$binary"
fi

dist="dist"
package_dir="$dist/tabmux-$version-$target"
rm -rf "$package_dir"
mkdir -p "$package_dir"

cp "$binary" "$package_dir/"
cp README.md LICENSE-MIT LICENSE-APACHE NOTICE RELEASE_NOTES.md "$package_dir/"
cp -R docs "$package_dir/"

echo "build-release: generating SBOM"
python3 scripts/make-sbom.py --output "$dist/SBOM-$version.cdx.json"

archive="$dist/tabmux-$version-$target.tar.gz"
rm -f "$archive"
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
  tar --sort=name --mtime='UTC 2020-01-01' --owner=0 --group=0 --numeric-owner \
    -czf "$archive" -C "$dist" "tabmux-$version-$target"
else
  tar -czf "$archive" -C "$dist" "tabmux-$version-$target"
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$@"
  else
    echo "build-release: no sha256sum or shasum available" >&2
    exit 1
  fi
}

(
  cd "$dist"
  sha256 "tabmux-$version-$target.tar.gz" "SBOM-$version.cdx.json" >SHA256SUMS
)

echo "build-release: artifacts"
ls -lh "$dist"
echo "build-release: checksums"
cat "$dist/SHA256SUMS"
