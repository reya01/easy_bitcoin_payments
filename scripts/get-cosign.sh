#!/usr/bin/env bash
# Download the signature checker (cosign) that ./scripts/update.sh uses, and
# check it against a fingerprint (sha256) that this project has pinned below.
#
#   ./scripts/get-cosign.sh                 # download into ./bin/cosign
#   ./scripts/get-cosign.sh --check         # CI: is the pinned fingerprint still right?
#   ./scripts/get-cosign.sh --print VERSION # maintainer: show the fingerprints of a version
#
# If the three values below are empty, nothing is pinned yet: this script then
# refuses to download anything (a download nobody checked is worth nothing), and
# you install cosign yourself instead:
#   https://docs.sigstore.dev/cosign/system_config/installation/
#
# MAINTAINER: to pin, run  ./scripts/get-cosign.sh --print v2.X.Y  on a computer
# with internet, compare each fingerprint with the one on that version's
# release page (https://github.com/sigstore/cosign/releases), then paste them here.
set -euo pipefail

COSIGN_VERSION=""
COSIGN_SHA256_AMD64=""
COSIGN_SHA256_ARM64=""

cd "$(dirname "$0")/.."
base="https://github.com/sigstore/cosign/releases/download"

arch_name() {
  case "$(uname -m)" in
    x86_64|amd64) echo amd64 ;;
    aarch64|arm64) echo arm64 ;;
    *) echo "unsupported computer type: $(uname -m)" >&2; return 1 ;;
  esac
}

fetch() { # version arch outfile
  curl -fsSL -m 120 "$base/$1/cosign-linux-$2" -o "$3"
}

mode="${1:-}"
case "$mode" in
  --print)
    v="${2:-}"
    [ -n "$v" ] || { echo "usage: $0 --print v2.X.Y" >&2; exit 1; }
    tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
    for a in amd64 arm64; do
      fetch "$v" "$a" "$tmp"
      echo "COSIGN_SHA256_${a^^}=\"$(sha256sum "$tmp" | cut -d' ' -f1)\""
    done
    echo "COSIGN_VERSION=\"$v\""
    exit 0
    ;;
esac

a="$(arch_name)"
if [ "$a" = amd64 ]; then want="$COSIGN_SHA256_AMD64"; else want="$COSIGN_SHA256_ARM64"; fi

if [ -z "$COSIGN_VERSION" ] || [ -z "$want" ]; then
  if [ "$mode" = "--check" ]; then
    echo "cosign is not pinned yet (nothing to check)."
    exit 0
  fi
  if [ "$mode" != "--quiet-if-unpinned" ]; then
    echo "No cosign version is pinned in this project yet, so nothing was downloaded." >&2
    echo "Install cosign yourself: https://docs.sigstore.dev/cosign/system_config/installation/" >&2
  fi
  exit 1
fi

tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
fetch "$COSIGN_VERSION" "$a" "$tmp"
got="$(sha256sum "$tmp" | cut -d' ' -f1)"
if [ "$got" != "$want" ]; then
  echo "cosign $COSIGN_VERSION does not match the pinned fingerprint." >&2
  echo "  wanted: $want" >&2
  echo "  got   : $got" >&2
  echo "It was NOT installed." >&2
  exit 1
fi
if [ "$mode" = "--check" ]; then
  echo "cosign $COSIGN_VERSION matches the pinned fingerprint."
  exit 0
fi
mkdir -p bin
install -m 0755 "$tmp" bin/cosign
echo "cosign $COSIGN_VERSION installed in ./bin/cosign (fingerprint checked)."
