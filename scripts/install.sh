#!/bin/sh
# Easy Bitcoin Payments (ebp) — development-only bare-binary installer.
#
# The supported deployment is the Docker one-liner on a VPS (public IP,
# DNS, TLS); this script is for local development and source builds.
#
# What it does:
#   1. downloads the release archive for your OS/arch,
#   2. verifies it against the release's SHA256SUMS file,
#   3. installs the `ebp` binary into /usr/local/bin (override: INSTALL_DIR),
#   4. creates the config and data directories,
#   5. prints the next steps (docs/RECIPE_SETUP.md).
#
# Safety: this script never executes remote shell code — there is no
# `curl … | sh` of anything. It downloads a binary archive and installs it
# only after its SHA256 matches the release's SHA256SUMS file. For full
# authenticity, also compare the SHA256SUMS file's own digest against the
# one published in the release notes before running this.
#
# TODO(releases): RELEASES_BASE below is a PLACEHOLDER — no public release
# has been tagged yet, so the download step will fail with an explanation.
# Until the first release, build from source:
#   git clone https://github.com/easy-bitcoin-payments/ebp && cargo build --release

set -eu

RELEASES_BASE="https://github.com/easy-bitcoin-payments/ebp/releases"
VERSION="${EBP_INSTALL_VERSION:-latest}"

INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
CONFIG_DIR="${CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/ebp}"
DATA_DIR="${DATA_DIR:-$HOME/.local/share/ebp}"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS:$ARCH" in
    Linux:x86_64) TARGET="linux-amd64" ;;
    Linux:aarch64) TARGET="linux-arm64" ;;
    Darwin:x86_64) TARGET="darwin-amd64" ;;
    Darwin:arm64) TARGET="darwin-arm64" ;;
    *)
        echo "error: unsupported platform: $OS $ARCH" >&2
        echo "build from source instead: cargo build --release (see the README)" >&2
        exit 1
        ;;
esac

fetch() {
    # fetch <url> <output-file>
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --retry 3 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        echo "error: need curl or wget to download the release" >&2
        exit 1
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo ""
    fi
}

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ASSET="ebp-${TARGET}.tar.gz"
if [ "$VERSION" = "latest" ]; then
    ARCHIVE_URL="${RELEASES_BASE}/latest/download/${ASSET}"
    SUMS_URL="${RELEASES_BASE}/latest/download/SHA256SUMS"
else
    ARCHIVE_URL="${RELEASES_BASE}/download/${VERSION}/${ASSET}"
    SUMS_URL="${RELEASES_BASE}/download/${VERSION}/SHA256SUMS"
fi

echo "Downloading ebp ${VERSION} for ${TARGET}:"
echo "  ${ARCHIVE_URL}"

if ! fetch "$ARCHIVE_URL" "$TMP_DIR/$ASSET"; then
    echo "" >&2
    echo "error: download failed: ${ARCHIVE_URL}" >&2
    echo "" >&2
    echo "The releases URL is a PLACEHOLDER until the first public release" >&2
    echo "is tagged (see the TODO at the top of this script). Until then," >&2
    echo "build from source:" >&2
    echo "  git clone https://github.com/easy-bitcoin-payments/ebp" >&2
    echo "  cd ebp && cargo build --release" >&2
    exit 1
fi

echo "Verifying checksum..."
fetch "$SUMS_URL" "$TMP_DIR/SHA256SUMS"
expected="$(grep " ${ASSET}\$" "$TMP_DIR/SHA256SUMS" | awk '{print $1}')"
if [ -z "$expected" ]; then
    echo "error: ${ASSET} is not listed in the release's SHA256SUMS" >&2
    exit 1
fi
actual="$(sha256_of "$TMP_DIR/$ASSET")"
if [ -z "$actual" ]; then
    echo "error: need sha256sum (or shasum) to verify the download" >&2
    exit 1
fi
if [ "$actual" != "$expected" ]; then
    echo "error: checksum mismatch — refusing to install" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
fi

tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"

BINARY="$TMP_DIR/ebp"
if [ ! -f "$BINARY" ]; then
    BINARY="$(find "$TMP_DIR" -type f -name ebp | head -n 1)"
fi
if [ -z "$BINARY" ] || [ ! -f "$BINARY" ]; then
    echo "error: the archive did not contain an ebp binary" >&2
    exit 1
fi

mkdir -p "$INSTALL_DIR" 2>/dev/null || true
if ! cp "$BINARY" "$INSTALL_DIR/ebp" 2>/dev/null; then
    echo "error: cannot install into ${INSTALL_DIR}" >&2
    echo "  re-run with a writable directory, e.g.:" >&2
    echo "  INSTALL_DIR=\$HOME/.local/bin sh scripts/install.sh" >&2
    exit 1
fi
chmod 0755 "$INSTALL_DIR/ebp"

mkdir -p "$CONFIG_DIR" "$DATA_DIR"

echo ""
echo "Installed:  $INSTALL_DIR/ebp"
echo "Config dir: $CONFIG_DIR (optional ebp.toml; see ebp.example.toml)"
echo "Data dir:   $DATA_DIR"
echo ""
echo "Next steps — follow the setup recipe:"
echo "  docs/RECIPE_SETUP.md"
echo ""
echo "Quick start (development stub wallet; see the recipe for signet):"
echo "  1. ebp serve                              # start the payments server"
echo "  2. ebp store create --name \"My store\"     # prints the store API key once"
echo "  3. for the supported VPS deploy, use the Docker one-liner in README.md"