#!/usr/bin/env bash
# Download the signature checker (cosign) that ./scripts/update.sh uses, and
# check it against a fingerprint (sha256) that this project has pinned below.
#
#   ./scripts/get-cosign.sh                 # download into ./bin/cosign
#   ./scripts/get-cosign.sh --check         # CI: is the pinned fingerprint still right?
#   ./scripts/get-cosign.sh --print VERSION # maintainer: show the fingerprints of a version
#   ./scripts/get-cosign.sh --verify-local  # is ./bin/cosign still the pinned file? (no internet)
#
# ./scripts/update.sh runs this by itself when cosign is missing, so nobody has to
# install anything by hand. If the three values below were ever empty, this script
# would refuse to download anything (a download nobody checked is worth nothing).
#
# MAINTAINER: to move to a newer cosign, run  ./scripts/get-cosign.sh --print v2.X.Y  on a computer
# with internet, compare each fingerprint with the one on that version's
# release page (https://github.com/sigstore/cosign/releases), then paste them here.
set -euo pipefail

# Taken from the cosign project's own installer (sigstore/cosign-installer, action.yml,
# "bootstrap" values for v3.1.4, which that project derived from its release's
# cosign_checksums.txt). CI downloads the real files and fails if they ever differ.
COSIGN_VERSION="v3.1.4"
COSIGN_SHA256_AMD64="424e2c97d0b45fd7a6b608a87f090f35633f4c23ece5f092d48bba11aaf4bbfa"
COSIGN_SHA256_ARM64="ade83b7aac62e131cacfe96d958948b2150cc9efec46a570a2649b8e27dedad2"

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

# Offline check of the copy already in ./bin: used by ./scripts/update.sh so
# it only trusts ./bin/cosign while its fingerprint still matches the pin.
if [ "$mode" = "--verify-local" ]; then
  [ -f bin/cosign ] || { echo "./bin/cosign is missing." >&2; exit 1; }
  got="$(sha256sum bin/cosign | cut -d' ' -f1)"
  if [ "$got" != "$want" ]; then
    echo "./bin/cosign does not match the pinned fingerprint for cosign $COSIGN_VERSION." >&2
    exit 1
  fi
  echo "./bin/cosign matches the pinned fingerprint (cosign $COSIGN_VERSION)."
  exit 0
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
