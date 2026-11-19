#!/usr/bin/env bash
# Checks scripts/update-helper.sh (and the status file update.sh writes for it) with
# fake docker, curl and cosign programs and a throwaway git repository: no real
# server, no network. The hard rules: a request is read strictly and used once,
# going backwards is refused with no way around it, only the exact digest the owner
# was shown can be installed, and every outcome leaves a clear status for EBP.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
fail() { echo "FAIL: $1" >&2; exit 1; }
rep() { head -c "$2" < /dev/zero | tr "\0" "$1"; }
DIGEST_OK="sha256:$(rep a 64)"
REF_OK="ghcr.io/reya01/easy_bitcoin_payments@$DIGEST_OK"
ID="0123456789abcdef0123"

new_sandbox() {
  work="$(mktemp -d)"
  mkdir -p "$work/origin.git" "$work/bin" "$work/repo/scripts"
  git init -q --bare "$work/origin.git"
  cd "$work/repo"
  git init -q
  git config user.email t@example.com; git config user.name t
  cp "$here/scripts/update.sh" "$here/scripts/update-helper.sh" scripts/
  printf '#!/usr/bin/env bash\necho "start image=${EBP_IMAGE:-} require=${EBP_REQUIRE_PREBUILT:-}" >> "%s/start.log"\n[ "${FAKE_START_FAILS:-}" = 1 ] && exit 1\nexit 0\n' "$work" > scripts/start.sh
  chmod +x scripts/*.sh
  printf 'updates/\n' > .gitignore
  git add -A; git commit -q -m one
  git tag v0.2.1-rc1
  git remote add origin "$work/origin.git"
  git push -q origin HEAD:refs/heads/main --tags 2>/dev/null
  sha="$(git rev-parse HEAD)"
  mkdir -p updates/inbox updates/status
  cat > "$work/bin/docker" <<'F'
#!/usr/bin/env bash
case "$*" in
  *"exec -T ebp curl"*) [ -n "${FAKE_RUNNING:-}" ] && echo "{\"status\":\"ok\",\"version\":\"$FAKE_RUNNING\"}"; exit 0 ;;
esac
exit 0
F
  cat > "$work/bin/curl" <<'F'
#!/usr/bin/env bash
case "$*" in *IMAGE_DIGEST*) [ -n "${FAKE_REF:-}" ] && echo "$FAKE_REF"; exit 0 ;; esac
exit 0
F
  cat > "$work/bin/cosign" <<'F'
#!/usr/bin/env bash
[ "${FAKE_COSIGN_FAIL:-}" = 1 ] && exit 1
echo "[{\"critical\":{},\"optional\":{\"githubWorkflowSha\":\"${FAKE_SHA}\",\"1.3.6.1.4.1.57264.1.21\":\"https://github.com/reya01/easy_bitcoin_payments/actions/runs/12345/attempts/1\"}}]"
F
  chmod +x "$work/bin/"*
  export FAKE_WORK="$work" FAKE_SHA="$sha" FAKE_REF="$REF_OK" FAKE_RUNNING="0.2.0-rc5"
  unset FAKE_COSIGN_FAIL FAKE_START_FAILS
}
done_sandbox() { cd /; rm -rf "$work"; }
request() { printf '%s' "$1" > updates/inbox/request.json; }
helper() { PATH="$work/bin:$PATH" ./scripts/update-helper.sh > /dev/null 2>&1 || true; }
status() { cat updates/status/status.json; }
field() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' updates/status/status.json "$1"; }
expect() { # expect STATE CODE what
  [ "$(field state)" = "$1" ] || fail "$3: state $(field state), wanted $1: $(status)"
  [ "$(field code)" = "$2" ] || fail "$3: code $(field code), wanted $2: $(status)"
  [ ! -e updates/inbox/request.json ] || fail "$3: the request was not removed"
}
no_change() {
  [ ! -f .env ] || ! grep -q '^EBP_IMAGE=' .env || fail "$1: .env was changed"
  [ ! -f "$work/start.log" ] || fail "$1: the server was restarted"
}

# 1. Garbage, a symlink and an oversized request are all refused, and removed.
new_sandbox
request 'not json'; helper; expect refused bad_request garbage
echo secret > "$work/target"; ln -s "$work/target" updates/inbox/request.json; helper
expect refused bad_request symlink; [ "$(cat "$work/target")" = secret ] || fail "symlink target touched"
request "{\"action\":\"check\",\"version\":\"0.2.1-rc1\",\"id\":\"$ID\",\"pad\":\"$(rep x 2000)\"}"; helper; expect refused bad_request oversized
request "{\"action\":\"rm -rf\",\"version\":\"0.2.1-rc1\",\"id\":\"$ID\"}"; helper; expect refused bad_request "unknown action"
request "{\"action\":\"check\",\"version\":\"0.2.1-rc1; reboot\",\"id\":\"$ID\"}"; helper; expect refused bad_request "bad version"
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"id\":\"$ID\"}"; helper; expect refused bad_request "apply without digest"
no_change garbage
[ -f updates/status/helper.json ] || fail "helper.json not written"
echo "ok  bad requests refused and removed (garbage, symlink, oversized, unknown action, bad version, no digest)"
done_sandbox

# 2. Check: a good release is shown with everything the owner needs, nothing changes.
new_sandbox
request "{\"action\":\"check\",\"version\":\"0.2.1-rc1\",\"id\":\"$ID\"}"; helper
expect ready ok check
[ "$(field running)" = 0.2.0-rc5 ] || fail "running version missing: $(status)"
[ "$(field version)" = 0.2.1-rc1 ] || fail "version missing"
[ "$(field digest)" = "$DIGEST_OK" ] || fail "digest missing"
[ "$(field commit)" = "$sha" ] || fail "commit missing"
[ "$(field request_id)" = "$ID" ] || fail "request id not echoed"
[ "$(field run_url)" = "https://github.com/reya01/easy_bitcoin_payments/actions/runs/12345/attempts/1" ] || fail "run url missing: $(status)"
no_change check
echo "ok  check shows running and new version, digest, commit and workflow run; changes nothing"
done_sandbox

# 3. Never backwards, never the same version, never when the running version is unknown.
new_sandbox; export FAKE_RUNNING=0.3.0
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"digest\":\"$DIGEST_OK\",\"id\":\"$ID\"}"; helper
expect refused older downgrade; no_change downgrade
export FAKE_RUNNING=0.2.1-rc1
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"digest\":\"$DIGEST_OK\",\"id\":\"$ID\"}"; helper
expect refused same_version same; no_change same
export FAKE_RUNNING=""
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"digest\":\"$DIGEST_OK\",\"id\":\"$ID\"}"; helper
expect refused no_running_version unknown; no_change unknown
echo "ok  older, same and unknown running versions are refused, with no override"
done_sandbox

# 4. update.sh itself refuses the escape hatches when run for the helper.
new_sandbox
out="$(PATH="$work/bin:$PATH" EBP_UPDATE_STATUS="$PWD/updates/status/status.json" ./scripts/update.sh --allow-unknown-running-version --check 0.2.1-rc1 2>&1 || true)"
printf '%s' "$out" | grep -q 'never skips the downgrade check' || fail "allow-unknown not refused in helper mode: $out"
[ "$(field code)" = not_allowed ] || fail "not_allowed not recorded"
out="$(PATH="$work/bin:$PATH" EBP_UPDATE_STATUS="$PWD/updates/status/status.json" ./scripts/update.sh --from-source --check 0.2.1-rc1 2>&1 || true)"
[ "$(field code)" = not_allowed ] || fail "from-source not refused in helper mode"
no_change hatches
echo "ok  helper mode has no downgrade override and no build-from-source"
done_sandbox

# 5. A bad signature is refused.
new_sandbox; export FAKE_COSIGN_FAIL=1
request "{\"action\":\"check\",\"version\":\"0.2.1-rc1\",\"id\":\"$ID\"}"; helper
expect refused signature_failed signature; no_change signature
echo "ok  bad signature refused"
done_sandbox

# 6. Only the digest the owner was shown can be installed.
new_sandbox
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"digest\":\"sha256:$(rep b 64)\",\"id\":\"$ID\"}"; helper
expect refused digest_changed digest; no_change digest
echo "ok  a different digest than the one approved is refused"
done_sandbox

# 7. Apply: the exact image is installed and the status says done.
new_sandbox
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"digest\":\"$DIGEST_OK\",\"id\":\"$ID\"}"; helper
expect done ok apply
grep -q "^EBP_IMAGE=$REF_OK$" .env || fail "exact image not stored"
grep -q "start image=$REF_OK require=1" "$work/start.log" || fail "server not started with the signed image"
echo "ok  apply installs exactly the approved signed image"
done_sandbox

# 8. A failed start is reported, and the old image setting is put back.
new_sandbox; export FAKE_START_FAILS=1
echo "EBP_IMAGE=old" > .env
request "{\"action\":\"apply\",\"version\":\"0.2.1-rc1\",\"digest\":\"$DIGEST_OK\",\"id\":\"$ID\"}"; helper
expect failed start_failed "failed start"
grep -q '^EBP_IMAGE=old$' .env || fail "old image setting not restored"
echo "ok  a failed start is reported and the old image setting restored"
done_sandbox

# 9. A symlink planted at status.json is replaced, not followed.
new_sandbox
echo keep > "$work/victim"; ln -s "$work/victim" updates/status/status.json
request 'junk'; helper
[ "$(cat "$work/victim")" = keep ] || fail "status write followed a symlink"
[ ! -L updates/status/status.json ] || fail "status.json is still a symlink"
echo "ok  status writes never follow a symlink"
done_sandbox

echo "update helper tests passed"
