#!/usr/bin/env bash
# Local release build — the same artifacts .github/workflows/release.yml
# produces when a v* tag is pushed, but built on your machine. Nothing
# here publishes anything: it builds the musl targets, writes
# dist/ebp-<target>.tar.gz and dist/SHA256SUMS, then prints the exact
# publish step CI performs.
#
# Usage:
#   scripts/release.sh                                  # both targets
#   scripts/release.sh x86_64-unknown-linux-musl        # one target
#
# aarch64 on an x86_64 host needs a cross toolchain — the workflow avoids
# that by building on a native arm64 runner; locally, either run this on
# an arm64 box or use `cross`:
#   cargo install cross
#   CROSS_BUILD=1 scripts/release.sh aarch64-unknown-linux-musl
set -euo pipefail

cd "$(dirname "$0")/.."

if [ $# -gt 0 ]; then
  TARGETS=("$@")
else
  TARGETS=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
fi

DIST="dist"
rm -rf "$DIST"
mkdir -p "$DIST"

for target in "${TARGETS[@]}"; do
  echo "==> building $target"
  rustup target add "$target"
  if [ "${CROSS_BUILD:-0}" = "1" ]; then
    cross build --release --target "$target"
  else
    cargo build --release --target "$target"
  fi
  # VERIFY: the crate's binary is `ebp` (as docker-compose uses).
  tar -czf "$DIST/ebp-$target.tar.gz" -C "target/$target/release" ebp
done

(cd "$DIST" && sha256sum ebp-*.tar.gz > SHA256SUMS)

echo
echo "Artifacts in $DIST/:"
ls -l "$DIST"
cat "$DIST/SHA256SUMS"
echo
echo "CI publishes these on a v* tag push (nothing here publishes):"
echo "  git tag v0.1.0 && git push origin v0.1.0"
echo "The workflow then creates the GitHub release (tarballs + SHA256SUMS)"
echo "and pushes ghcr.io/<owner>/<repo>:v0.1.0 (no moving 'latest' tag, on purpose)"
echo "for amd64+arm64, signed by digest."