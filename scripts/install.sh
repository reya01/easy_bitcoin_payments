#!/usr/bin/env bash
# Install the EBP release binary from GitHub releases (bare-binary path —
# the supported deploy is the Docker one-liner; this is for advanced
# self-hosters). Detects the architecture, downloads the matching musl
# tarball, checks that SHA256SUMS was signed by this project's release
# workflow for that exact tag (sigstore, with a cosign whose fingerprint is
# pinned below), verifies the tarball against SHA256SUMS, and installs `ebp`
# into ~/.local/bin (override with EBP_INSTALL_DIR). Nothing to install first.
#
# Usage:
#   scripts/install.sh              # latest release
#   scripts/install.sh 0.1.0        # a specific release tag (v optional)
set -euo pipefail

REPO="reya01/easy_bitcoin_payments"
OIDC_ISSUER="https://token.actions.githubusercontent.com"
# The signature checker, pinned. Must equal scripts/get-cosign.sh (CI checks).
COSIGN_VERSION="v3.1.4"
COSIGN_SHA256_AMD64="424e2c97d0b45fd7a6b608a87f090f35633f4c23ece5f092d48bba11aaf4bbfa"

case "$(uname -s).$(uname -m)" in
  Linux.x86_64 | Linux.amd64) TARGET="x86_64-unknown-linux-musl" ;;
  *)
    echo "error: unsupported platform $(uname -s) $(uname -m) — the release"
    echo "ships an x86_64 Linux musl binary only for now (ARM support is planned"
    echo "for v1.0); on anything else build from source:"
    echo "https://github.com/${REPO}"
    exit 1
    ;;
esac

VERSION="${1:-latest}"
CURL=(curl -fsSL --retry 3 --connect-timeout 10 --max-time 300)
if [ "$VERSION" = "latest" ]; then
  # Find which tag "latest" is, so the signature can be checked for that tag.
  url="$("${CURL[@]}" -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
  TAG="${url##*/tag/}"
  case "$TAG" in v[0-9]*) ;; *) echo "error: could not work out the latest release (got: $url)" >&2; exit 1 ;; esac
else
  TAG="v${VERSION#v}"
fi
# Real release URL pattern:
# https://github.com/reya01/easy_bitcoin_payments/releases/download/vTAG/ebp-<target>.tar.gz
BASE="https://github.com/${REPO}/releases/download/${TAG}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> downloading ebp ${TAG} (${TARGET})"
"${CURL[@]}" -o "$TMP/ebp-$TARGET.tar.gz" "$BASE/ebp-$TARGET.tar.gz"
"${CURL[@]}" -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS"
"${CURL[@]}" -o "$TMP/SHA256SUMS.sigstore.json" "$BASE/SHA256SUMS.sigstore.json" \
  || { echo "error: ${TAG} has no signature file (SHA256SUMS.sigstore.json), so it was not installed" >&2; exit 1; }

echo "==> getting the signature checker (cosign ${COSIGN_VERSION}, fingerprint pinned)"
"${CURL[@]}" -o "$TMP/cosign" "https://github.com/sigstore/cosign/releases/download/${COSIGN_VERSION}/cosign-linux-amd64"
got="$(sha256sum "$TMP/cosign" | cut -d' ' -f1)"
[ "$got" = "$COSIGN_SHA256_AMD64" ] \
  || { echo "error: the downloaded cosign does not match the pinned fingerprint; nothing was installed" >&2; exit 1; }
chmod +x "$TMP/cosign"

echo "==> checking the signature (asks the public Sigstore service)"
"$TMP/cosign" verify-blob --bundle "$TMP/SHA256SUMS.sigstore.json" \
  --certificate-identity "https://github.com/${REPO}/.github/workflows/release.yml@refs/tags/${TAG}" \
  --certificate-oidc-issuer "$OIDC_ISSUER" \
  "$TMP/SHA256SUMS" \
  || { echo "error: SHA256SUMS was NOT signed by this project's release workflow for ${TAG}; nothing was installed" >&2; exit 1; }

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