#!/usr/bin/env bash
# Install the EBP release binary from GitHub releases (bare-binary path —
# the supported deploy is the Docker one-liner; this is for advanced
# self-hosters). Detects the architecture, downloads the matching musl
# tarball, verifies it against SHA256SUMS, and installs `ebp` into
# ~/.local/bin (override with EBP_INSTALL_DIR).
#
# Usage:
#   scripts/install.sh              # latest release
#   scripts/install.sh 0.1.0        # a specific release tag (v optional)
set -euo pipefail

REPO="reya01/easy_bitcoin_payments"

case "$(uname -s).$(uname -m)" in
  Linux.x86_64 | Linux.amd64) TARGET="x86_64-unknown-linux-musl" ;;
  Linux.aarch64 | Linux.arm64) TARGET="aarch64-unknown-linux-musl" ;;
  *)
    echo "error: unsupported platform $(uname -s) $(uname -m) — the release"
    echo "ships x86_64 and aarch64 Linux musl binaries; on anything else"
    echo "build from source: https://github.com/${REPO}"
    exit 1
    ;;
esac

VERSION="${1:-latest}"
if [ "$VERSION" = "latest" ]; then
  BASE="https://github.com/${REPO}/releases/latest/download"
else
  # Real release URL pattern:
  # https://github.com/reya01/easy_bitcoin_payments/releases/download/vTAG/ebp-<target>.tar.gz
  BASE="https://github.com/${REPO}/releases/download/v${VERSION#v}"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> downloading ebp ${VERSION} (${TARGET})"
curl -fsSL --retry 3 --connect-timeout 10 --max-time 300 -o "$TMP/ebp-$TARGET.tar.gz" "$BASE/ebp-$TARGET.tar.gz"
curl -fsSL --retry 3 --connect-timeout 10 --max-time 300 -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS"

echo "==> verifying checksum"
# Fixed-string, guarded: a SHA256SUMS missing our artifact (renamed asset,
# partial release) must fail with a clear message, not a silent set -e exit.
grep -F "ebp-$TARGET.tar.gz" "$TMP/SHA256SUMS" > "$TMP/sum" \
  || { echo "error: no checksum line for ebp-$TARGET.tar.gz in SHA256SUMS" >&2; exit 1; }
(cd "$TMP" && sha256sum -c sum)

echo "==> installing"
BIN_DIR="${EBP_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$BIN_DIR"
tar -xzf "$TMP/ebp-$TARGET.tar.gz" -C "$TMP"
install -m 0755 "$TMP/ebp" "$BIN_DIR/ebp"

echo "installed: $BIN_DIR/ebp"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not on your PATH" ;;
esac
echo "next: the supported deploy is the Docker one-liner (barkd sidecar +"
echo "TLS); the bare binary is documented for advanced self-hosters."