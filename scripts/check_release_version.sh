#!/usr/bin/env bash
# Refuse to cut a release when the tag does not match the files.
#
# Usage: scripts/check_release_version.sh v0.2.0-rc3
#
# Checks that the tag (without the leading "v") equals the version in
# Cargo.toml, Cargo.lock, the WooCommerce plugin header, its
# EBP_BITCOIN_VERSION constant and its readme "Stable tag", and that
# CHANGELOG.md has a heading for it. The release workflow runs this first on
# every tag, so a tag pushed before the version bump is merged fails at once,
# before anything is built, published or signed.
set -euo pipefail

tag="${1:-}"
if [ -z "$tag" ]; then
  echo "usage: $0 vTAG (for example v0.2.0-rc3)" >&2
  exit 2
fi
want="${tag#v}"

cd "$(dirname "$0")/.."

cargo_toml=$(awk '/^\[package\]/{p=1} p && /^version[[:space:]]*=/{gsub(/"/,"",$3); print $3; exit}' Cargo.toml)
cargo_lock=$(awk '/^name = "easy-bitcoin-payments"$/{f=1; next} f && /^version = /{gsub(/"/,"",$3); print $3; exit}' Cargo.lock)
plugin=integrations/woocommerce/ebp-bitcoin
plugin_header=$(sed -n 's/^ \* Version: *//p' "$plugin/ebp-bitcoin.php" | head -1 | tr -d '[:space:]')
plugin_const=$(sed -n "s/.*define( 'EBP_BITCOIN_VERSION', '\\([^']*\\)'.*/\\1/p" "$plugin/ebp-bitcoin.php" | head -1)
plugin_readme=$(sed -n 's/^Stable tag: *//p' "$plugin/readme.txt" | head -1 | tr -d '[:space:]')

bad=0
check() { # name value
  if [ "$2" != "$want" ]; then
    echo "MISMATCH: $1 is '$2' but the tag says '$want'" >&2
    bad=1
  fi
}
check "Cargo.toml version" "$cargo_toml"
check "Cargo.lock version" "$cargo_lock"
check "plugin header Version" "$plugin_header"
check "plugin EBP_BITCOIN_VERSION" "$plugin_const"
check "plugin readme Stable tag" "$plugin_readme"
if ! grep -q "^## ${want}[[:space:]]" CHANGELOG.md; then
  echo "MISMATCH: CHANGELOG.md has no '## ${want} ...' heading" >&2
  bad=1
fi

if [ "$bad" -ne 0 ]; then
  echo "Not releasing. Merge the version bump first, then tag the commit that contains it." >&2
  exit 1
fi
echo "OK: tag $tag matches Cargo.toml, Cargo.lock, the plugin files and CHANGELOG.md"
