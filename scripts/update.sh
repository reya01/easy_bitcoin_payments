#!/usr/bin/env bash
# Update this server to a release of Easy Bitcoin Payments, after checking it.
#
# Run it yourself, from the folder that holds docker-compose.yml, when YOU
# decide to update. Nothing in EBP ever runs it for you (no timer, no
# auto-update).
#
#   ./scripts/update.sh 0.2.1-rc1              # signed image, about a minute
#   ./scripts/update.sh --check 0.2.1-rc1      # only check it, change nothing
#   ./scripts/update.sh --from-source 0.2.1-rc1  # build it yourself (slow, fully independent)
#
# The update helper (scripts/update-helper.sh) runs this same script when the owner
# presses Update on the admin page's Home tab. It sets EBP_UPDATE_STATUS so each
# result is also written as a small status file, and passes --expect-digest so only
# the exact image the owner was shown can be installed.
#
# What it does, in order:
#   1. works out which version you run now and REFUSES to go backwards
#   2. downloads the release's image address (a "digest": the exact file)
#   3. checks the cryptographic signature with cosign: the image must have been
#      built and signed by this project's release workflow for exactly that tag
#   4. shows you the version, the digest and the result, and asks you to type yes
#   5. only then switches the server to that exact image
#
# Read this file before you run it: it is the one piece of the update that
# comes from `git` and not from the signed release (see docs/UPGRADING.md).
#
# Why it is all inside functions: the script switches the folder to the
# release's files part-way through, and bash reads a script as it runs, so the
# whole file is read before anything changes.
set -euo pipefail

REPO="reya01/easy_bitcoin_payments"
IMAGE_REPO="ghcr.io/${REPO}"
OIDC_ISSUER="https://token.actions.githubusercontent.com"
VERSION_RE='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'

say() { printf '%s\n' "$*"; }

# For the update helper: when EBP_UPDATE_STATUS names a file, every outcome is also
# written there as one line of JSON (fixed state and code words, plus version numbers,
# the digest and the commit, each checked against a strict pattern first). Written to a
# temporary file and renamed, so a reader never sees half a file and a symlink at the
# target is replaced, not followed.
STATUS_FILE="${EBP_UPDATE_STATUS:-}"
REQUEST_ID="${EBP_UPDATE_REQUEST_ID:-}"
S_RUNNING=""; S_VERSION=""; S_DIGEST=""; S_COMMIT=""; S_RUN_URL=""
only() { printf '%s' "$2" | grep -Eq "$1" && printf '%s' "$2" || true; }
status() {
  [ -n "$STATUS_FILE" ] || return 0
  local state="$1" code="${2:-}" dir tmp
  dir="$(dirname "$STATUS_FILE")"
  tmp="$(mktemp "$dir/.status.XXXXXX")" || return 0
  printf '{"state":"%s","code":"%s","request_id":"%s","running":"%s","version":"%s","digest":"%s","commit":"%s","run_url":"%s","at":%s}\n' \
    "$(only '^[a-z_]+$' "$state")" "$(only '^[a-z_]+$' "$code")" "$(only '^[0-9a-f]{16,64}$' "$REQUEST_ID")" \
    "$(only "$VERSION_RE" "$S_RUNNING")" "$(only "$VERSION_RE" "$S_VERSION")" \
    "$(only '^sha256:[0-9a-f]{64}$' "$S_DIGEST")" "$(only '^[0-9a-f]{40}$' "$S_COMMIT")" \
    "$(only "^https://github\\.com/${REPO}/actions/runs/[0-9]+(/attempts/[0-9]+)?$" "$S_RUN_URL")" \
    "$(date +%s)" > "$tmp"
  chmod 0644 "$tmp"
  mv -f -T "$tmp" "$STATUS_FILE"
}
# die "message" [code]: stop, say nothing changed, and record why (code) for the helper.
die() { status refused "${2:-error}"; printf 'Stopped: %s\nNothing was changed.\n' "$1" >&2; exit 1; }

# Compare two versions like 0.2.0-rc5 and 0.2.1: prints -1, 0 or 1.
# A version with a "-something" part is OLDER than the same number without it
# (0.2.0-rc5 is older than 0.2.0), as the semver rules say.
version_cmp() {
  local a="$1" b="$2" ca cb pa pb
  ca="${a%%-*}"; cb="${b%%-*}"
  pa=""; pb=""
  case "$a" in *-*) pa="${a#*-}" ;; esac
  case "$b" in *-*) pb="${b#*-}" ;; esac
  local IFS=.
  local -a xa xb
  read -r -a xa <<< "$ca"
  read -r -a xb <<< "$cb"
  local i
  for i in 0 1 2; do
    if [ "${xa[$i]:-0}" -lt "${xb[$i]:-0}" ]; then echo -1; return; fi
    if [ "${xa[$i]:-0}" -gt "${xb[$i]:-0}" ]; then echo 1; return; fi
  done
  if [ -z "$pa" ] && [ -z "$pb" ]; then echo 0; return; fi
  if [ -z "$pa" ]; then echo 1; return; fi
  if [ -z "$pb" ]; then echo -1; return; fi
  local -a ya yb
  read -r -a ya <<< "$pa"
  read -r -a yb <<< "$pb"
  local n=${#ya[@]}
  if [ "${#yb[@]}" -gt "$n" ]; then n=${#yb[@]}; fi
  for ((i = 0; i < n; i++)); do
    local sa="${ya[$i]-}" sb="${yb[$i]-}"
    if [ -z "$sa" ]; then echo -1; return; fi
    if [ -z "$sb" ]; then echo 1; return; fi
    local na=0 nb=0
    case "$sa" in *[!0-9]*) na=1 ;; esac
    case "$sb" in *[!0-9]*) nb=1 ;; esac
    if [ "$na" = 0 ] && [ "$nb" = 0 ]; then
      if [ "$sa" -lt "$sb" ]; then echo -1; return; fi
      if [ "$sa" -gt "$sb" ]; then echo 1; return; fi
    elif [ "$na" = 0 ]; then echo -1; return      # numbers sort before words
    elif [ "$nb" = 0 ]; then echo 1; return
    else
      # Words like rc9 and rc10: compare the word, then the number as a number
      # (strict semver would put rc10 before rc9; people expect the opposite).
      local wa="${sa%%[0-9]*}" wb="${sb%%[0-9]*}" da="${sa#"${sa%%[0-9]*}"}" db="${sb#"${sb%%[0-9]*}"}"
      if [[ "$wa" < "$wb" ]]; then echo -1; return; fi
      if [[ "$wa" > "$wb" ]]; then echo 1; return; fi
      case "$da$db" in *[!0-9]*) da=0; db=0 ;; esac
      if [ "${da:-0}" -lt "${db:-0}" ]; then echo -1; return; fi
      if [ "${da:-0}" -gt "${db:-0}" ]; then echo 1; return; fi
    fi
  done
  echo 0
}

# Write or replace KEY=VALUE in .env (docker compose reads it automatically).
set_env() {
  touch .env
  grep -v "^$1=" .env > .env.new || true
  echo "$1=$2" >> .env.new
  mv .env.new .env
}
unset_env() {
  [ -f .env ] || return 0
  grep -v "^$1=" .env > .env.new || true
  mv .env.new .env
}
get_env() { [ -f .env ] && grep "^$1=" .env | tail -n 1 | cut -d= -f2- || true; }

# The version the server is running right now (from its own health answer).
running_version() {
  local body=""
  body="$(docker compose exec -T ebp curl -fsS -m 5 http://127.0.0.1:8080/api/v1/health 2>/dev/null || true)"
  printf '%s' "$body" | sed -n 's/.*"version" *: *"\([^"]*\)".*/\1/p' | head -n 1
}

# The signature checker. Only this project's pinned copy (./bin/cosign) is
# trusted first, and only while its fingerprint still matches the pin; a
# cosign found elsewhere on this computer is used only as a last resort.
pinned_cosign_ok() {
  [ -x ./bin/cosign ] && [ -x ./scripts/get-cosign.sh ] \
    && ./scripts/get-cosign.sh --verify-local >/dev/null 2>&1
}

usage() {
  sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'
}

main() {
  cd "$(dirname "$0")/.."

  local check_only="" assume_yes="" from_source="" allow_unknown="" version="" expect_digest=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --check) check_only=1 ;;
      --yes) assume_yes=1 ;;
      --from-source) from_source=1 ;;
      --allow-unknown-running-version) allow_unknown=1 ;;
      --expect-digest)
        shift
        [ "$#" -gt 0 ] || die "--expect-digest needs a value like sha256:..." bad_request
        printf '%s' "$1" | grep -Eq '^sha256:[0-9a-f]{64}$' || die "--expect-digest must look like sha256: and 64 hex characters" bad_request
        expect_digest="$1" ;;
      -h|--help) usage; exit 0 ;;
      -*) die "unknown option $1 (try --help)" bad_request ;;
      *) [ -z "$version" ] || die "give one version only" bad_request; version="${1#v}" ;;
    esac
    shift
  done

  # Run by the update helper: no escape hatches. Going backwards stays impossible even
  # when the running version cannot be read, and only the signed image is used.
  if [ -n "$STATUS_FILE" ]; then
    [ -z "$allow_unknown" ] || die "the update helper never skips the downgrade check" not_allowed
    [ -z "$from_source" ] || die "the update helper only installs the signed image" not_allowed
  fi

  if [ -z "$version" ]; then
    status refused missing_version
    say "Which version? For example:  ./scripts/update.sh 0.2.1-rc1"
    say "The versions are listed on https://github.com/${REPO}/releases"
    say "(Release candidates are never picked for you: you choose.)"
    exit 1
  fi
  printf '%s' "$version" | grep -Eq "$VERSION_RE" || die "'$version' does not look like a version (like 0.2.1 or 0.2.1-rc1)" bad_version
  local tag="v$version"
  S_VERSION="$version"

  # 1. Never go backwards.
  local now
  now="$(running_version)"
  S_RUNNING="$now"
  if [ -z "$now" ]; then
    if [ -z "$allow_unknown" ]; then
      die "could not read the version this server runs now (is it running? try ./scripts/start.sh first). To go ahead anyway, add --allow-unknown-running-version" no_running_version
    fi
    say "Warning: could not read the running version, so the downgrade check was skipped."
  else
    local cmp
    cmp="$(version_cmp "$version" "$now")"
    if [ "$cmp" = 0 ]; then
      status refused same_version
      say "This server already runs $now. Nothing to do."
      exit 0
    fi
    if [ "$cmp" = -1 ]; then
      die "$version is OLDER than the $now this server runs. EBP does not go backwards (an older version refuses to start on newer data). If you really need an older version, restore a backup (docs/UPGRADING.md, 'The safety net')." older
    fi
  fi

  if [ -n "$from_source" ]; then
    from_source_update "$version" "$tag" "$now" "$check_only" "$assume_yes"
    return
  fi

  # The released image is built for x86-64 computers only for now.
  case "$(uname -m)" in
    x86_64|amd64) ;;
    *) die "the signed image is only built for x86-64 computers, and this one is $(uname -m). Build it yourself instead: ./scripts/update.sh --from-source $version" arch ;;
  esac

  # 2. The exact image for this release.
  local ref
  ref="$(curl -fsSL -m 30 --max-filesize 2000 "https://github.com/${REPO}/releases/download/${tag}/IMAGE_DIGEST" 2>/dev/null | head -n 1 | tr -d '\r' || true)"
  [ -n "$ref" ] || die "could not download the image address for $tag. Does that release exist (https://github.com/${REPO}/releases/tag/${tag}), and does this server have internet?" no_digest
  printf '%s' "$ref" | grep -Eq "^${IMAGE_REPO//./\\.}@sha256:[0-9a-f]{64}$" || die "the image address for $tag is not in the expected form (${IMAGE_REPO}@sha256:...), so it was not used" bad_digest
  local digest="${ref#*@}"
  S_DIGEST="$digest"
  if [ -n "$expect_digest" ] && [ "$digest" != "$expect_digest" ]; then
    die "the image for $tag is now $digest, not the $expect_digest you were shown. Check again before updating." digest_changed
  fi

  # 3. The signature.
  local cosign=""
  if pinned_cosign_ok; then
    cosign=./bin/cosign
  else
    say "Getting the signature checker (cosign) once, checked against a pinned fingerprint..."
    if [ -x ./scripts/get-cosign.sh ] && ./scripts/get-cosign.sh >/dev/null 2>&1 && pinned_cosign_ok; then
      cosign=./bin/cosign
    elif command -v cosign >/dev/null 2>&1; then
      cosign="$(command -v cosign)"
      say "Could not get the pinned signature checker, so using the cosign already installed on this computer ($cosign)."
    else
      die "the signature checker 'cosign' could not be set up automatically (it needs internet). Install it yourself (https://docs.sigstore.dev/cosign/system_config/installation/) and run this again." no_cosign
    fi
  fi
  local identity="https://github.com/${REPO}/.github/workflows/release.yml@refs/tags/${tag}"
  say "Checking the signature (this asks the public Sigstore service, so it needs internet)..."
  local out
  if ! out="$("$cosign" verify "$ref" \
        --certificate-identity "$identity" \
        --certificate-oidc-issuer "$OIDC_ISSUER" 2>/dev/null)"; then
    die "the signature check FAILED for $ref. This image was not signed by this project's release workflow for $tag. It was not used. If you are sure a release exists, try again in a few minutes; otherwise do not update and tell the project." signature_failed
  fi
  # The files in this folder must come from the very commit the image was built from.
  git fetch -q --tags origin 2>/dev/null || die "could not fetch the release files from GitHub" fetch_failed
  local want_sha built_sha
  want_sha="$(git rev-parse -q --verify "refs/tags/${tag}^{commit}" || true)"
  built_sha="$(printf '%s' "$out" | sed -n 's/.*"githubWorkflowSha" *: *"\([0-9a-f]\{40\}\)".*/\1/p' | head -n 1)"
  [ -n "$want_sha" ] || die "the release files for $tag are not on GitHub (no such tag)" no_tag
  [ -n "$built_sha" ] || die "the signature did not say which commit built the image, so the files could not be matched to it" no_commit
  [ "$want_sha" = "$built_sha" ] || die "the tag $tag points at a different commit than the one the image was built from. Something is wrong; do not update." commit_mismatch
  S_COMMIT="$built_sha"
  S_RUN_URL="$(printf '%s' "$out" | grep -Eo "https://github\.com/${REPO}/actions/runs/[0-9]+(/attempts/[0-9]+)?" | head -n 1 || true)"

  # 4. Show everything, then ask.
  say
  say "================================================================"
  say "  Update EBP"
  say "    running now : ${now:-unknown}"
  say "    update to   : $version"
  say "    image       : $digest"
  say "    signature   : OK. Built and signed by this project's release"
  say "                  workflow for $tag (commit ${built_sha:0:12})"
  say "    release     : https://github.com/${REPO}/releases/tag/${tag}"
  say "================================================================"
  say "A signature shows who built it, not that it is bug-free. Read the release"
  say "notes first, and download a fresh backup (Settings) before you continue."
  if [ -n "$check_only" ]; then
    say
    status ready ok
    say "Check only: nothing was changed."
    exit 0
  fi
  if [ -z "$assume_yes" ]; then
    [ -t 0 ] || die "this needs a person to type yes (or add --yes)" needs_person
    printf 'Type yes to update now: '
    local answer=""
    read -r answer
    [ "$answer" = "yes" ] || die "you did not type yes" not_confirmed
  fi
  status updating ok

  # 5. Switch. The folder's files move to the release's own files first.
  git -c advice.detachedHead=false checkout -q "$tag" || die "could not switch this folder to the release files (do you have local changes? run: git status)" checkout_failed
  local previous
  previous="$(get_env EBP_IMAGE)"
  set_env EBP_IMAGE "$ref"
  export EBP_IMAGE="$ref"
  if ! EBP_REQUIRE_PREBUILT=1 ./scripts/start.sh; then
    if [ -n "$previous" ]; then set_env EBP_IMAGE "$previous"; else unset_env EBP_IMAGE; fi
    say "The update did not finish. The image setting was put back." >&2
    say "Your data was not touched. Run ./scripts/start.sh to start the version you had." >&2
    status failed start_failed
    exit 1
  fi
  status "done" ok
  say
  say "Updated to $version. Check that the admin Home page says the wallet is connected."
  say "To go back to your previous server image if something looks wrong: restore your backup first (docs/UPGRADING.md), then ask for help."
  if [ -z "$STATUS_FILE" ] && [ ! -f /etc/systemd/system/ebp-update-helper.path ] && [ -x ./scripts/install-update-helper.sh ]; then
    say
    say "Tip: to update from the admin page's Home tab next time instead of here, run once:"
    say "  ./scripts/install-update-helper.sh"
  fi
}

# Build from the source files of a release: slower, but nothing is downloaded
# except the source and the build's own packages.
from_source_update() {
  local version="$1" tag="$2" now="$3" check_only="$4" assume_yes="$5"
  git fetch -q --tags origin 2>/dev/null || die "could not fetch the release files from GitHub"
  git rev-parse -q --verify "refs/tags/${tag}^{commit}" >/dev/null || die "the release files for $tag are not on GitHub (no such tag)"
  say "Build $version from source, on this server."
  say "  running now : ${now:-unknown}"
  say "  source      : $tag ($(git rev-parse --short "refs/tags/${tag}^{commit}"))"
  say "This takes several minutes (10 or more on a small server). No signed image is used."
  if [ -n "$check_only" ]; then say "Check only: nothing was changed."; exit 0; fi
  if [ -z "$assume_yes" ]; then
    [ -t 0 ] || die "this needs a person to type yes (or add --yes)"
    printf 'Type yes to build and update now: '
    local answer=""
    read -r answer
    [ "$answer" = "yes" ] || die "you did not type yes"
  fi
  git -c advice.detachedHead=false checkout -q "$tag" || die "could not switch this folder to the release files (do you have local changes? run: git status)"
  unset_env EBP_IMAGE
  unset EBP_IMAGE
  ./scripts/start.sh
  say "Updated to $version (built here)."
}

main "$@"
exit 0
