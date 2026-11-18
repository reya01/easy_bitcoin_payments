#!/usr/bin/env bash
# Build the `ebp` binary the way anyone can repeat, and print its sha256.
#
# The idea: the same source, built with the same tools, gives the same bytes on any machine.
# So this script
#   * uses ONLY committed files (git archive of HEAD), never your working folder;
#   * builds inside the exact Rust image the Dockerfile uses (pinned by digest, so the compiler,
#     linker and C compiler are identical everywhere);
#   * builds in a fixed folder (/src) and rewrites the folder names inside the binary;
#   * uses --locked (Cargo.lock is law) and turns off incremental builds;
#   * sets SOURCE_DATE_EPOCH to the commit's time, for anything that wants a clock.
#
# Usage:   scripts/repro-build.sh [OUTPUT_DIR]        (default: dist-repro)
# Needs:   docker, git, and a clean checkout of the commit you want to check
#          (uncommitted changes are NOT built; the script says so).
# Output:  OUTPUT_DIR/ebp, OUTPUT_DIR/ebp.sha256, OUTPUT_DIR/build-info.txt
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="${1:-dist-repro}"

IMAGE="$(grep -m1 '^FROM rust:' Dockerfile | awk '{print $2}')"
[ -n "$IMAGE" ] || { echo "error: could not read the pinned Rust image from the Dockerfile" >&2; exit 1; }
COMMIT="$(git rev-parse HEAD)"
SDE="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}"

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "note: you have uncommitted changes; they are NOT part of this build (only commit $COMMIT is)." >&2
fi

rm -rf "$OUT"
mkdir -p "$OUT"
OUT_ABS="$(cd "$OUT" && pwd)"

echo "==> building commit ${COMMIT:0:12} in $IMAGE"
git archive --format=tar HEAD | docker run --rm -i \
  -e SOURCE_DATE_EPOCH="$SDE" \
  -e CARGO_INCREMENTAL=0 \
  -e CARGO_TERM_COLOR=never \
  -v "$OUT_ABS":/out \
  "$IMAGE" bash -euc '
    mkdir /src && tar -x -C /src && cd /src
    export RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=${CARGO_HOME:-/usr/local/cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-/usr/local/rustup}=/rustup"
    cargo build --release --locked
    cp target/release/ebp /out/ebp
    {
      echo "rustc:   $(rustc --version)"
      echo "cargo:   $(cargo --version)"
      echo "gcc:     $(gcc --version | head -1)"
      echo "ld:      $(ld --version | head -1)"
      echo "epoch:   $SOURCE_DATE_EPOCH"
    } > /out/build-info.txt
  '

( cd "$OUT_ABS" && sha256sum ebp > ebp.sha256 )
{ echo "commit:  $COMMIT"; echo "image:   $IMAGE"; cat "$OUT_ABS/build-info.txt"; } > "$OUT_ABS/build-info.tmp" && mv "$OUT_ABS/build-info.tmp" "$OUT_ABS/build-info.txt"

echo
cat "$OUT_ABS/build-info.txt"
echo
cat "$OUT_ABS/ebp.sha256"
