#!/usr/bin/env bash
# The EBP update helper: lets the owner update from the admin page's Home tab
# without a console, while EBP itself never gets control of the server.
#
# It runs as root, started by systemd (deploy/systemd/ebp-update-helper.path)
# whenever EBP drops a request file. It has no network port and accepts exactly
# two requests, both written by EBP after the owner pressed a button:
#
#   {"action":"check","version":"0.2.2-rc1","id":"<hex>"}
#       -> runs  ./scripts/update.sh --check VERSION          (changes nothing)
#   {"action":"apply","version":"0.2.2-rc1","digest":"sha256:<64 hex>","id":"<hex>"}
#       -> runs  ./scripts/update.sh --yes --expect-digest DIGEST VERSION
#
# Every safety check lives in update.sh, unchanged: the cosign signature must come
# from this project's release workflow for exactly that tag, the tag's commit must
# be the commit that built the image, and a version older than or equal to the
# running one is refused, with no way around it. --expect-digest makes sure only
# the exact image the owner was shown can be installed.
#
# The request is never trusted: it must be a small regular file (not a symlink),
# and only the action word, a version number, a digest and an id that match strict
# patterns are taken from it. Nothing from it is ever run or used as a path.
#
# Folders (made by scripts/install-update-helper.sh):
#   updates/inbox   written by EBP (container user 10001), read here
#   updates/status  written here (root), mounted read-only into EBP
#
# Why it is all inside functions: update.sh switches this folder to the release's
# own files part-way through, which replaces this file too, and bash reads a
# script as it runs. Wrapping everything in main makes bash read it all first.
set -euo pipefail

HELPER_VERSION=1

main() {
  cd "$(dirname "$0")/.."
  local dir="${EBP_UPDATES_DIR:-$PWD/updates}"
  local inbox="$dir/inbox" outdir="$dir/status"
  local req="$inbox/request.json" status_file="$outdir/status.json"
  [ -d "$inbox" ] && [ -d "$outdir" ] || { echo "update helper: $inbox or $outdir is missing (run scripts/install-update-helper.sh)" >&2; exit 1; }

  # One run at a time.
  exec 9>"$outdir/.lock"
  flock -n 9 || exit 0

  # Tell EBP the helper is installed, and which version.
  write_json "$outdir/helper.json" "{\"helper_version\":$HELPER_VERSION,\"at\":$(date +%s)}"

  [ -e "$req" ] || [ -L "$req" ] || exit 0

  # Read the request strictly, then remove it before acting, so a request is used once.
  local parsed=""
  parsed="$(parse_request "$req" || true)"
  rm -f -- "$req"

  local action="" version="" digest="" id=""
  if [ -n "$parsed" ]; then
    action="$(printf '%s\n' "$parsed" | sed -n 1p)"
    version="$(printf '%s\n' "$parsed" | sed -n 2p)"
    digest="$(printf '%s\n' "$parsed" | sed -n 3p)"
    id="$(printf '%s\n' "$parsed" | sed -n 4p)"
  fi

  local log="$outdir/last-run.log"
  case "$action" in
    check)
      write_status "$status_file" checking ok "$id" "$version"
      EBP_UPDATE_STATUS="$status_file" EBP_UPDATE_REQUEST_ID="$id" \
        ./scripts/update.sh --check "$version" > "$log" 2>&1 || true
      ;;
    apply)
      write_status "$status_file" checking ok "$id" "$version"
      EBP_UPDATE_STATUS="$status_file" EBP_UPDATE_REQUEST_ID="$id" \
        ./scripts/update.sh --yes --expect-digest "$digest" "$version" > "$log" 2>&1 < /dev/null || true
      ;;
    *)
      write_status "$status_file" refused bad_request "$id" ""
      exit 0
      ;;
  esac
  chmod 0644 "$log" 2>/dev/null || true

  # update.sh writes the final status. If it stopped without one (it crashed),
  # do not leave EBP showing "checking" or "updating" for ever.
  if grep -Eq '"state":"(checking|updating)"' "$status_file" 2>/dev/null; then
    write_status "$status_file" failed unexpected "$id" "$version"
  fi
}

# Prints four lines (action, version, digest, id) when the request is valid; nothing otherwise.
parse_request() {
  python3 -I - "$1" <<'PY'
import json, os, re, stat, sys
VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
ID = re.compile(r"^[0-9a-f]{16,64}$")
try:
    fd = os.open(sys.argv[1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode) or st.st_size > 1024:
            sys.exit(1)
        data = os.read(fd, 1025)
    finally:
        os.close(fd)
    d = json.loads(data.decode("utf-8"))
    if not isinstance(d, dict):
        sys.exit(1)
    action, version, rid = d.get("action"), d.get("version"), d.get("id")
    digest = d.get("digest", "")
    if action not in ("check", "apply"):
        sys.exit(1)
    if not isinstance(version, str) or len(version) > 64 or not VERSION.match(version):
        sys.exit(1)
    if not isinstance(rid, str) or not ID.match(rid):
        sys.exit(1)
    if action == "apply":
        if not isinstance(digest, str) or not DIGEST.match(digest):
            sys.exit(1)
    else:
        digest = ""
    print(action); print(version); print(digest); print(rid)
except SystemExit:
    raise
except Exception:
    sys.exit(1)
PY
}

# Write a file by temp-and-rename, so a symlink at the target is replaced, not followed.
write_json() {
  local target="$1" body="$2" tmp
  tmp="$(mktemp "$(dirname "$target")/.tmp.XXXXXX")"
  printf '%s\n' "$body" > "$tmp"
  chmod 0644 "$tmp"
  mv -f -T "$tmp" "$target"
}

write_status() {
  local file="$1" state="$2" code="$3" id="$4" version="$5"
  printf '%s' "$id" | grep -Eq '^[0-9a-f]{16,64}$' || id=""
  printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || version=""
  write_json "$file" "{\"state\":\"$state\",\"code\":\"$code\",\"request_id\":\"$id\",\"running\":\"\",\"version\":\"$version\",\"digest\":\"\",\"commit\":\"\",\"run_url\":\"\",\"at\":$(date +%s)}"
}

main "$@"
exit 0
