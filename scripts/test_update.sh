#!/usr/bin/env bash
# Checks ./scripts/update.sh with fake docker, curl and cosign programs and a
# throwaway git repository (no real server, no network). The point: it must
# refuse to go backwards, refuse a bad digest or signature, change nothing on
# --check, and only on a full success switch the server to the exact image.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
fail() { echo "FAIL: $1" >&2; exit 1; }

rep() { head -c "$2" < /dev/zero | tr "\0" "$1"; }
REF_OK="ghcr.io/reya01/easy_bitcoin_payments@sha256:$(rep a 64)"

# Build a fresh sandbox: a git repo with a bare "origin" holding tag v0.2.1-rc1.
new_sandbox() {
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  mkdir -p "$work/origin.git" "$work/bin" "$work/repo/scripts"
  git init -q --bare "$work/origin.git"
  cd "$work/repo"
  git init -q
  git config user.email t@example.com; git config user.name t
  cp "$here/scripts/update.sh" scripts/
  printf '#!/usr/bin/env bash\necho "start image=${EBP_IMAGE:-} require=${EBP_REQUIRE_PREBUILT:-} head=$(git rev-parse --short HEAD)" >> "%s/start.log"\n[ "${FAKE_START_FAILS:-}" = 1 ] && exit 1\nexit 0\n' "$work" > scripts/start.sh
  chmod +x scripts/*.sh
  echo one > file.txt
  git add -A; git commit -q -m one
  git tag v0.2.1-rc1
  git remote add origin "$work/origin.git"
  git push -q origin HEAD:refs/heads/main --tags 2>/dev/null
  sha="$(git rev-parse HEAD)"
  cat > "$work/bin/docker" <<'F'
#!/usr/bin/env bash
case "$*" in
  *"exec -T ebp curl"*)
    [ -n "${FAKE_RUNNING:-}" ] && echo "{\"status\":\"ok\",\"version\":\"$FAKE_RUNNING\"}"
    exit 0 ;;
esac
exit 0
F
  cat > "$work/bin/curl" <<'F'
#!/usr/bin/env bash
case "$*" in
  *IMAGE_DIGEST*) [ -n "${FAKE_REF:-}" ] && echo "$FAKE_REF"; exit 0 ;;
esac
exit 0
F
  cat > "$work/bin/cosign" <<'F'
#!/usr/bin/env bash
echo "$*" >> "$FAKE_WORK/cosign.log"
[ "${FAKE_COSIGN_FAIL:-}" = 1 ] && exit 1
echo "[{\"critical\":{},\"optional\":{\"githubWorkflowSha\":\"${FAKE_SHA}\"}}]"
F
  chmod +x "$work/bin/"*
  export FAKE_WORK="$work" FAKE_SHA="$sha" FAKE_REF="$REF_OK" FAKE_RUNNING="0.2.0-rc5"
  unset FAKE_COSIGN_FAIL FAKE_START_FAILS
}
run() { PATH="$work/bin:$PATH" ./scripts/update.sh "$@" 2>&1 || true; }
no_change() { # what: .env untouched and start.sh never ran
  [ ! -f .env ] || ! grep -q '^EBP_IMAGE=' .env || fail "$1: .env was changed"
  [ ! -f "$work/start.log" ] || fail "$1: the server was restarted"
}

# 1. going backwards is refused
new_sandbox; export FAKE_RUNNING="0.3.0"
out="$(run --yes 0.2.1-rc1)"
printf '%s' "$out" | grep -q 'OLDER' || fail "downgrade not refused: $out"
no_change downgrade; echo "ok  downgrade refused"
rm -rf "$work"; trap - EXIT

# 2. already running this version
new_sandbox; export FAKE_RUNNING="0.2.1-rc1"
out="$(run --yes 0.2.1-rc1)"
printf '%s' "$out" | grep -q 'already runs' || fail "same version not noticed: $out"
no_change same; echo "ok  same version is a no-op"
rm -rf "$work"; trap - EXIT

# 3. no version / bad version
new_sandbox
out="$(run)"; printf '%s' "$out" | grep -q 'Which version' || fail "missing version not explained"
out="$(run --yes 'latest')"; printf '%s' "$out" | grep -q 'does not look like a version' || fail "bad version accepted"
no_change version; echo "ok  version is required and checked"
rm -rf "$work"; trap - EXIT

# 4. unreadable running version is refused unless allowed
new_sandbox; export FAKE_RUNNING=""
out="$(run --yes 0.2.1-rc1)"; printf '%s' "$out" | grep -q 'could not read the version' || fail "unknown running version not refused: $out"
no_change unknown; echo "ok  unknown running version refused"
rm -rf "$work"; trap - EXIT

# 5. an image address from somewhere else is refused
new_sandbox; export FAKE_REF="ghcr.io/someone/else@sha256:$(rep a 64)"
out="$(run --yes 0.2.1-rc1)"; printf '%s' "$out" | grep -q 'not in the expected form' || fail "foreign image accepted: $out"
no_change foreign; echo "ok  foreign image refused"
rm -rf "$work"; trap - EXIT

# 6. no cosign available
new_sandbox; rm "$work/bin/cosign"
out="$(PATH="$work/bin:$PATH" ./scripts/update.sh --yes 0.2.1-rc1 2>&1 || true)"
printf '%s' "$out" | grep -q 'cosign' || fail "missing cosign not explained: $out"
no_change nocosign; echo "ok  missing cosign refused"
rm -rf "$work"; trap - EXIT

# 7. a failed signature is refused
new_sandbox; export FAKE_COSIGN_FAIL=1
out="$(run --yes 0.2.1-rc1)"; printf '%s' "$out" | grep -q 'signature check FAILED' || fail "bad signature accepted: $out"
no_change badsig; echo "ok  bad signature refused"
rm -rf "$work"; trap - EXIT

# 8. signed by a different commit than the tag is refused
new_sandbox; FAKE_SHA="$(rep b 40)"; export FAKE_SHA
out="$(run --yes 0.2.1-rc1)"; printf '%s' "$out" | grep -q 'different commit' || fail "commit mismatch accepted: $out"
no_change commit; echo "ok  commit mismatch refused"
rm -rf "$work"; trap - EXIT

# 9. --check shows the facts and changes nothing; cosign got the exact identity
new_sandbox
out="$(run --check 0.2.1-rc1)"
printf '%s' "$out" | grep -q 'signature   : OK' || fail "--check did not show the signature result: $out"
printf '%s' "$out" | grep -q "${REF_OK#*@}" || fail "--check did not show the digest"
printf '%s' "$out" | grep -q 'running now : 0.2.0-rc5' || fail "--check did not show the running version"
grep -q -- '--certificate-identity https://github.com/reya01/easy_bitcoin_payments/.github/workflows/release.yml@refs/tags/v0.2.1-rc1 ' "$work/cosign.log" || fail "cosign identity is not the exact tag: $(cat "$work/cosign.log")"
grep -q -- '--certificate-oidc-issuer https://token.actions.githubusercontent.com' "$work/cosign.log" || fail "cosign issuer missing"
no_change check; echo "ok  --check changes nothing, shows version, digest and signature"
rm -rf "$work"; trap - EXIT

# 10. without --yes and without a person at the keyboard it refuses
new_sandbox
out="$(run 0.2.1-rc1 </dev/null)"; printf '%s' "$out" | grep -q 'needs a person to type yes' || fail "ran without a yes: $out"
no_change noyes; echo "ok  never updates without a yes"
rm -rf "$work"; trap - EXIT

# 11. full success switches to the exact image, with a signed image required
new_sandbox
out="$(run --yes 0.2.1-rc1)"
printf '%s' "$out" | grep -q 'Updated to 0.2.1-rc1' || fail "success not reported: $out"
grep -qx "EBP_IMAGE=$REF_OK" .env || fail ".env does not hold the exact image"
grep -q "require=1" "$work/start.log" || fail "start.sh was not told to require the signed image"
grep -q "image=$REF_OK" "$work/start.log" || fail "start.sh did not get the image"
[ "$(git rev-parse HEAD)" = "$sha" ] || fail "folder is not at the release tag"
echo "ok  success: exact image stored, signed image required"
rm -rf "$work"; trap - EXIT

# 12. a failed start puts the old setting back
new_sandbox; export FAKE_START_FAILS=1
echo "EBP_IMAGE=ghcr.io/reya01/easy_bitcoin_payments@sha256:$(rep c 64)" > .env
out="$(run --yes 0.2.1-rc1)"
printf '%s' "$out" | grep -q 'did not finish' || fail "failed start not reported: $out"
grep -qx "EBP_IMAGE=ghcr.io/reya01/easy_bitcoin_payments@sha256:$(rep c 64)" .env || fail "old image setting not restored"
echo "ok  failed start restores the old image setting"
rm -rf "$work"; trap - EXIT

# 13. building from source: only the tag, never a signed image, no downgrade
new_sandbox
echo "EBP_IMAGE=$REF_OK" > .env
out="$(run --from-source --yes 0.2.1-rc1)"
printf '%s' "$out" | grep -q 'built here' || fail "source update not reported: $out"
! grep -q '^EBP_IMAGE=' .env || fail "source build still has a signed image set"
[ ! -f "$work/cosign.log" ] || fail "source build used cosign"
export FAKE_RUNNING="0.3.0"
out="$(run --from-source --yes 0.2.1-rc1)"; printf '%s' "$out" | grep -q 'OLDER' || fail "source build allowed a downgrade"
echo "ok  --from-source builds the tag and also refuses downgrades"

# 14. start.sh: a chosen image is only ever downloaded, never quietly built
sandbox2="$(mktemp -d)"; trap 'rm -rf "$sandbox2"' EXIT
mkdir -p "$sandbox2/scripts" "$sandbox2/bin"
cp "$here/scripts/start.sh" "$sandbox2/scripts/"
cat > "$sandbox2/bin/docker" <<'F'
#!/usr/bin/env bash
echo "$*" >> "$FAKE_LOG"
case "$*" in "compose pull"*) exit "${FAKE_PULL_EXIT:-0}" ;; "compose logs"*) echo "SETUP CODE: ABCD-1234" ;; esac
exit 0
F
printf '#!/usr/bin/env bash\nexit 0\n' > "$sandbox2/bin/curl"
chmod +x "$sandbox2/bin/"* "$sandbox2/scripts/start.sh"
sed -i 's/seq 1 60/seq 1 1/; s/sleep 2/true/' "$sandbox2/scripts/start.sh"
export FAKE_LOG="$sandbox2/docker.log"
cd "$sandbox2"
echo "EBP_IMAGE=$REF_OK" > .env
FAKE_PULL_EXIT=1 PATH="$sandbox2/bin:$PATH" ./scripts/start.sh >/dev/null 2>&1 && fail "start.sh went on after a failed download"
! grep -q 'compose build' "$FAKE_LOG" || fail "start.sh built from source after a failed download"
: > "$FAKE_LOG"
PATH="$sandbox2/bin:$PATH" ./scripts/start.sh >/dev/null 2>&1
grep -q 'compose pull ebp' "$FAKE_LOG" || fail "image from .env was not pulled"
! grep -q 'compose build' "$FAKE_LOG" || fail "built although an image was chosen"
rm .env; : > "$FAKE_LOG"
PATH="$sandbox2/bin:$PATH" ./scripts/start.sh >/dev/null 2>&1
grep -q 'compose build ebp' "$FAKE_LOG" || fail "no image chosen: should build from source as before"
rm -f .env
EBP_REQUIRE_PREBUILT=1 PATH="$sandbox2/bin:$PATH" ./scripts/start.sh >/dev/null 2>&1 && fail "required a signed image but built anyway"
echo "ok  start.sh never swaps a failed download for a build"
echo "update.sh tests passed"
