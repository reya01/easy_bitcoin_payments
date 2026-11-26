#!/usr/bin/env bash
# Build the release download (ebp-x86_64-unknown-linux-musl.tar.gz) the way anyone can repeat,
# and print its sha256. The release workflow publishes exactly this file, so anyone with Docker
# can rebuild a tag and compare it with the release's SHA256SUMS.
#
# Same idea as scripts/repro-build.sh, plus what the musl download needs:
#   * builds inside the digest-pinned Rust image from the Dockerfile (same compiler everywhere);
#   * the musl C tools come from a DATED Debian snapshot at an exact version, not "whatever
#     Debian serves today" (they compile the bundled SQLite and ring);
#   * the musl standard library comes from rustup for that image's exact Rust version;
#   * the tarball stores fixed names, owner, order and time (the commit's time), and gzip
#     stores no name or time.
#
# Usage:   scripts/repro-build-musl.sh [OUTPUT_DIR]          (default: dist-repro-musl)
# Needs:   docker, git, network access (Debian snapshot and rustup), a checkout of the commit.
# Output:  OUTPUT_DIR/ebp-x86_64-unknown-linux-musl.tar.gz, OUTPUT_DIR/SHA256SUMS.part,
#          OUTPUT_DIR/build-info.txt
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="${1:-dist-repro-musl}"
TARGET="x86_64-unknown-linux-musl"
NAME="ebp-$TARGET.tar.gz"
# Debian snapshot date and exact musl version. Change both together, on purpose, in a PR.
SNAPSHOT="20261101T000000Z"
MUSL_VERSION="1.2.3-1"

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

echo "==> building $NAME from commit ${COMMIT:0:12} in $IMAGE"
git archive --format=tar HEAD | docker run --rm -i \
  -e SOURCE_DATE_EPOCH="$SDE" \
  -e CARGO_INCREMENTAL=0 \
  -e CARGO_TERM_COLOR=never \
  -e SNAPSHOT="$SNAPSHOT" -e MUSL_VERSION="$MUSL_VERSION" -e TARGET="$TARGET" -e NAME="$NAME" \
  -v "$OUT_ABS":/out \
  "$IMAGE" bash -euc '
    rm -f /etc/apt/sources.list.d/* /etc/apt/sources.list
    echo "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/$SNAPSHOT bookworm main" \
      > /etc/apt/sources.list
    apt-get -qq update
    apt-get -qq install -y --no-install-recommends "musl=$MUSL_VERSION" "musl-dev=$MUSL_VERSION" "musl-tools=$MUSL_VERSION" >/dev/null
    rustup target add "$TARGET" >/dev/null 2>&1
    mkdir /src && tar -x -C /src && cd /src
    export RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=${CARGO_HOME:-/usr/local/cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-/usr/local/rustup}=/rustup"
    cargo build --release --locked --target "$TARGET"
    tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
        --format=gnu --mode="u=rwx,go=rx" -C "target/$TARGET/release" -cf - ebp \
      | gzip -n -9 > "/out/$NAME"
    {
      echo "rustc:   $(rustc --version)"
      echo "cargo:   $(cargo --version)"
      echo "gcc:     $(gcc --version | head -1)"
      echo "musl:    $(dpkg-query -W -f="\${Version}" musl-tools) (Debian snapshot $SNAPSHOT)"
      echo "epoch:   $SOURCE_DATE_EPOCH"
      echo "binary:  $(sha256sum target/$TARGET/release/ebp | cut -d" " -f1)"
    } > /out/build-info.txt
  '

( cd "$OUT_ABS" && sha256sum "$NAME" > SHA256SUMS.part )
{ echo "commit:  $COMMIT"; echo "image:   $IMAGE"; cat "$OUT_ABS/build-info.txt"; } > "$OUT_ABS/build-info.tmp" && mv "$OUT_ABS/build-info.tmp" "$OUT_ABS/build-info.txt"

echo
cat "$OUT_ABS/build-info.txt"
echo
cat "$OUT_ABS/SHA256SUMS.part"
