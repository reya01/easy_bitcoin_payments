#!/usr/bin/env bash
# Check a published EBP release against its source code.
#
# It rebuilds the `ebp` program from the release's tag with scripts/repro-build.sh, takes the
# `ebp` program out of the published Docker image (the amd64 one), and compares the two. If they
# match, the published image runs exactly the program that source code produces, and you did not
# have to trust whoever built it. For tags that have scripts/repro-build-musl.sh it also rebuilds
# the download (ebp-x86_64-unknown-linux-musl.tar.gz) and compares it with the release's
# SHA256SUMS. See docs/dev/REPRODUCIBLE_BUILDS.md.
#
# Usage:   scripts/verify-release.sh VERSION            (for example 0.3.3)
#          scripts/verify-release.sh VERSION IMAGE      (IMAGE = ghcr.io/...@sha256:..., if you
#                                                        already have it from the release notes)
# Needs:   docker, git, curl, and a clone of this repository with its tags (git fetch --tags).
# Note:    only releases built after the Dockerfile took the script's settings (stage 2) can
#          match; older releases are reported as different.
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="reya01/easy_bitcoin_payments"
VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "usage: scripts/verify-release.sh VERSION [IMAGE]   (for example 0.3.3)" >&2
  exit 2
fi
TAG="v${VERSION#v}"
IMAGE="${2:-}"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || {
  echo "error: this clone has no tag $TAG. Run: git fetch --tags" >&2
  exit 1
}

if [ -z "$IMAGE" ]; then
  echo "==> reading the image fingerprint published with $TAG"
  IMAGE="$(curl -fsSL "https://github.com/$REPO/releases/download/$TAG/IMAGE_DIGEST" | tr -d '[:space:]')"
fi
case "$IMAGE" in
  ghcr.io/*@sha256:*) ;;
  *) echo "error: not an image fingerprint: '$IMAGE'" >&2; exit 1 ;;
esac

WORK="$(mktemp -d)"
cleanup() {
  git worktree remove --force "$WORK/src" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "==> rebuilding $TAG from source (this takes a while)"
git worktree add --detach "$WORK/src" "$TAG" >/dev/null
"$WORK/src/scripts/repro-build.sh" "$WORK/rebuilt" >/dev/null
rebuilt="$(cut -d' ' -f1 "$WORK/rebuilt/ebp.sha256")"

echo "==> taking the program out of $IMAGE"
docker pull -q --platform linux/amd64 "$IMAGE" >/dev/null
id="$(docker create --platform linux/amd64 "$IMAGE")"
docker cp "$id":/usr/local/bin/ebp "$WORK/ebp-published" >/dev/null
docker rm "$id" >/dev/null
published="$(sha256sum "$WORK/ebp-published" | cut -d' ' -f1)"

echo
echo "rebuilt from $TAG:   $rebuilt"
echo "inside the image:    $published"
status=0
if [ "$rebuilt" = "$published" ]; then
  echo "MATCH: the published image runs exactly the program this source code builds."
else
  echo "DIFFERENT: the published image's program is not what this source code builds."
  echo "(Releases built before the Dockerfile used the same settings as the script are expected"
  echo "to differ. For a newer release, please report it: see SECURITY.md.)"
  status=1
fi

# The download (ebp-x86_64-unknown-linux-musl.tar.gz), if this tag can build it repeatably.
if [ -x "$WORK/src/scripts/repro-build-musl.sh" ]; then
  echo
  echo "==> rebuilding the download (ebp-x86_64-unknown-linux-musl.tar.gz) from $TAG"
  "$WORK/src/scripts/repro-build-musl.sh" "$WORK/rebuilt-musl" >/dev/null
  tar_rebuilt="$(cut -d' ' -f1 "$WORK/rebuilt-musl/SHA256SUMS.part")"
  tar_published="$(curl -fsSL "https://github.com/$REPO/releases/download/$TAG/SHA256SUMS" \
    | awk '$2 == "ebp-x86_64-unknown-linux-musl.tar.gz" {print $1}')"
  echo "download rebuilt from $TAG:  $tar_rebuilt"
  echo "download in SHA256SUMS:      $tar_published"
  if [ -n "$tar_published" ] && [ "$tar_rebuilt" = "$tar_published" ]; then
    echo "MATCH: the published download is exactly what this source code builds."
  else
    echo "DIFFERENT: the published download is not what this source code builds. Please report it: see SECURITY.md."
    status=1
  fi
else
  echo
  echo "(This tag is older than the repeatable download build, so only the image was checked.)"
fi
exit "$status"
