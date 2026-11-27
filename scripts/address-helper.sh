#!/usr/bin/env bash
# The EBP address helper: lets the owner move to their own domain from the admin
# page (Settings, Your address) without a console, while EBP itself never gets
# control of the server. See docs/OWN_DOMAIN.md.
#
# It runs as root, started by systemd (deploy/systemd/ebp-address-helper.path)
# whenever EBP drops updates/inbox/address-request.json. It has no network port
# and accepts exactly one kind of request, written by EBP after the owner pressed
# a button (or, for giving up the backup, after 30 good days):
#
#   {"action":"address","main":"pay.myshop.com","others":["203-0-113-7.sslip.io"],"id":"<hex>"}
#
# It then serves exactly those names: it sets EBP_DOMAIN, EBP_SITE_NAMES,
# EBP_PUBLIC_BASE_URL and EBP_PUBLIC_ALIASES in .env and restarts the web server
# and EBP so they use them. Every check and decision is made by EBP; this helper
# only refuses anything that is not a plain host name (checked again here), at
# most three names, no duplicates. Nothing from the request is ever run or used
# as a path.
set -euo pipefail

HELPER_VERSION=1

main() {
  cd "$(dirname "$0")/.."
  local dir="${EBP_UPDATES_DIR:-$PWD/updates}"
  local inbox="$dir/inbox" outdir="$dir/status"
  local req="$inbox/address-request.json" status_file="$outdir/address-status.json"
  [ -d "$inbox" ] && [ -d "$outdir" ] || { echo "address helper: $inbox or $outdir is missing (run scripts/install-update-helper.sh)" >&2; exit 1; }

  exec 9>"$outdir/.address-lock"
  flock -n 9 || exit 0

  write_json "$outdir/address-helper.json" "{\"helper_version\":$HELPER_VERSION,\"at\":$(date +%s)}"

  [ -e "$req" ] || [ -L "$req" ] || exit 0
  local parsed=""
  parsed="$(parse_request "$req" || true)"
  rm -f -- "$req"
  if [ -z "$parsed" ]; then
    write_json "$status_file" "{\"state\":\"failed\",\"code\":\"bad_request\",\"id\":\"\",\"at\":$(date +%s)}"
    exit 0
  fi

  local id main names aliases
  id="$(printf '%s\n' "$parsed" | sed -n 1p)"
  main="$(printf '%s\n' "$parsed" | sed -n 2p)"
  names="$(printf '%s\n' "$parsed" | sed -n 3p)"
  aliases="$(printf '%s\n' "$parsed" | sed -n 4p)"

  write_json "$status_file" "{\"state\":\"working\",\"code\":\"ok\",\"id\":\"$id\",\"at\":$(date +%s)}"
  set_env EBP_DOMAIN "$main"
  set_env EBP_SITE_NAMES "$names"
  set_env EBP_PUBLIC_BASE_URL "https://$main"
  set_env EBP_PUBLIC_ALIASES "$aliases"

  # Recreate only what reads these settings: the web server and EBP.
  if docker compose up -d --no-deps caddy ebp > "$outdir/address-last-run.log" 2>&1; then
    write_json "$status_file" "{\"state\":\"done\",\"code\":\"ok\",\"id\":\"$id\",\"at\":$(date +%s)}"
  else
    write_json "$status_file" "{\"state\":\"failed\",\"code\":\"restart_failed\",\"id\":\"$id\",\"at\":$(date +%s)}"
  fi
  chmod 0644 "$outdir/address-last-run.log" 2>/dev/null || true
}

# Write or replace KEY=VALUE in .env (temp file, then rename).
set_env() {
  touch .env
  local tmp
  tmp="$(mktemp .env.XXXXXX)"
  grep -v "^$1=" .env > "$tmp" || true
  printf '%s=%s\n' "$1" "$2" >> "$tmp"
  chmod --reference=.env "$tmp" 2>/dev/null || chmod 0600 "$tmp"
  mv -f -T "$tmp" .env
}

# Prints four lines (id, main, site names, aliases) when the request is valid;
# nothing otherwise.
parse_request() {
  python3 -I - "$1" <<'PY'
import json, os, re, stat, sys
LABEL = r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"
HOST = re.compile(r"^(?:%s\.)+[a-z](?:[a-z0-9-]{0,61}[a-z0-9])?$" % LABEL)
ID = re.compile(r"^[0-9a-f]{16,64}$")
try:
    fd = os.open(sys.argv[1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode) or st.st_size > 2048:
            sys.exit(1)
        data = os.read(fd, 2049)
    finally:
        os.close(fd)
    d = json.loads(data.decode("utf-8"))
    if not isinstance(d, dict) or d.get("action") != "address":
        sys.exit(1)
    rid, main, others = d.get("id"), d.get("main"), d.get("others")
    if not isinstance(rid, str) or not ID.match(rid):
        sys.exit(1)
    if not isinstance(others, list) or len(others) > 2:
        sys.exit(1)
    names = [main] + others
    for n in names:
        if not isinstance(n, str) or len(n) > 253 or not HOST.match(n):
            sys.exit(1)
    if len(set(names)) != len(names):
        sys.exit(1)
    print(rid)
    print(main)
    print(", ".join(names))
    print(",".join("https://" + n for n in others))
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

main "$@"
exit 0
