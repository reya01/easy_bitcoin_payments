#!/usr/bin/env bash
# Release plumbing for Easy Bitcoin Payments (Phase 1 close-out).
#
# Usage: scripts/release.sh <version>   (e.g. scripts/release.sh v0.1.2)
#
# What it does, in order:
#   1. builds the release binary (host target)
#   2. writes dist/SHA256SUMS with the binary's checksum
#   3. builds and tags the Docker image
#   4. prints (does not run) the push commands — the registry URL is not
#      decided yet; fill in REGISTRY below when it is.
#
# Multi-arch (x86_64 + arm64) cross-builds and the signed-tag check land
# with the Phase 3 hardening pass; this script is the single place they
# will live.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-}"
if [ -z "${VERSION}" ]; then
  echo "usage: scripts/release.sh <version>   (e.g. scripts/release.sh v0.1.2)" >&2
  exit 1
fi

# Fill in when the registry is decided, e.g.:
#   REGISTRY="ghcr.io/easy-bitcoin-payments"
REGISTRY=""

echo "==> cargo build --release"
cargo build --release

mkdir -p dist
cp -f target/release/ebp "dist/ebp-${VERSION}"

echo "==> dist/SHA256SUMS"
( cd dist && sha256sum "ebp-${VERSION}" > SHA256SUMS )
cat dist/SHA256SUMS

echo "==> docker build"
docker build -t "ebp:${VERSION}" -t ebp:latest .

if [ -n "${REGISTRY}" ]; then
  echo "==> docker tag"
  docker tag "ebp:${VERSION}" "${REGISTRY}/ebp:${VERSION}"
  docker tag "ebp:${VERSION}" "${REGISTRY}/ebp:latest"
  # Review before pushing: the tag must point at the reviewed commit.
  # docker push "${REGISTRY}/ebp:${VERSION}"
  # docker push "${REGISTRY}/ebp:latest"
else
  echo "==> docker push skipped: REGISTRY is not set (see the comment at the top of this script)"
fi

echo "==> done. Attach dist/ebp-${VERSION} and dist/SHA256SUMS to the release."