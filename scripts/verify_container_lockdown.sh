#!/usr/bin/env bash
# Checks that the running EBP container really is locked down, from the inside:
#
#   ./scripts/verify_container_lockdown.sh               # safe on any running server
#   ./scripts/verify_container_lockdown.sh --with-barkd  # CI: also start the wallet program
#                                                        # under the same limits (empty folder)
#
# What it checks (docker-compose.yml sets all of it):
#   - the program runs as an ordinary user, not root
#   - it has no special powers (no Linux "capabilities") and cannot gain any
#   - the container's files are read-only: writing to the program folder fails
#   - it can still write where it must: the data folder and the scratch folder
#   - the scratch folder cannot run programs
# Nothing is changed on a real server: the only things written are small test files
# that are removed again. --with-barkd starts barkd in a throwaway folder inside /data,
# which is for CI; do not use it on a server with a wallet you care about.
#
# Run it from the folder that holds docker-compose.yml.
set -uo pipefail
cd "$(dirname "$0")/.."

with_barkd=""
[ "${1:-}" = "--with-barkd" ] && with_barkd=1

fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1" >&2; fails=$((fails + 1)); }
inside() { docker compose exec -T ebp sh -c "$1" 2>&1; }

cid="$(docker compose ps -q ebp 2>/dev/null | head -n 1)"
if [ -z "$cid" ]; then
  echo "The server is not running here. Start it first: ./scripts/start.sh" >&2
  exit 2
fi
echo "Checking the EBP container ($(docker inspect --format '{{.Config.Image}}' "$cid" | cut -c1-70))"

# From docker's own record of how it was started.
ro="$(docker inspect --format '{{.HostConfig.ReadonlyRootfs}}' "$cid")"
[ "$ro" = true ] && ok "files are read-only" || bad "the container's files are not read-only (ReadonlyRootfs=$ro)"
drop="$(docker inspect --format '{{range .HostConfig.CapDrop}}{{.}} {{end}}' "$cid")"
case " $drop " in *" ALL "*|*" all "*) ok "all Linux capabilities dropped" ;; *) bad "capabilities are not all dropped (CapDrop=$drop)" ;; esac
sec="$(docker inspect --format '{{range .HostConfig.SecurityOpt}}{{.}} {{end}}' "$cid")"
case "$sec" in *no-new-privileges*) ok "cannot gain new privileges (setting)" ;; *) bad "no-new-privileges is not set (SecurityOpt=$sec)" ;; esac

# From inside: what the running program really has.
uid="$(inside 'id -u' | tr -d '\r')"
[ "$uid" = 10001 ] && ok "runs as user $uid (not root)" || bad "runs as user '$uid', expected 10001"
caps="$(inside "grep '^CapEff' /proc/1/status | awk '{print \$2}'" | tr -d '\r')"
[ "$caps" = 0000000000000000 ] && ok "no capabilities in the running program" || bad "running program has capabilities: $caps"
nnp="$(inside "grep '^NoNewPrivs' /proc/1/status | awk '{print \$2}'" | tr -d '\r')"
[ "$nnp" = 1 ] && ok "cannot gain new privileges (running program)" || bad "NoNewPrivs is '$nnp', expected 1"

# Writes: blocked where they should be, allowed where they must be.
if inside 'touch /usr/local/bin/.lockdown-test' >/dev/null; then
  inside 'rm -f /usr/local/bin/.lockdown-test' >/dev/null
  bad "could write into the program folder"
else
  ok "writing to the program folder fails"
fi
if inside 'touch /home/ebp/.lockdown-test' >/dev/null; then
  inside 'rm -f /home/ebp/.lockdown-test' >/dev/null
  bad "could write into the home folder"
else
  ok "writing to the home folder fails"
fi
if inside 'touch /data/.lockdown-test && rm -f /data/.lockdown-test' >/dev/null; then
  ok "can write to the data folder"
else
  bad "cannot write to the data folder (the server could not save anything)"
fi
if inside 'touch /tmp/.lockdown-test && rm -f /tmp/.lockdown-test' >/dev/null; then
  ok "can write to the scratch folder"
else
  bad "cannot write to the scratch folder"
fi
if inside 'cp /bin/true /tmp/.lockdown-run && /tmp/.lockdown-run' >/dev/null; then
  inside 'rm -f /tmp/.lockdown-run' >/dev/null
  bad "a program could be run from the scratch folder"
else
  inside 'rm -f /tmp/.lockdown-run' >/dev/null
  ok "programs cannot run from the scratch folder"
fi

# CI only: the wallet program (barkd) must start and answer under these limits, in an
# empty throwaway folder. It writes only inside /data.
if [ -n "$with_barkd" ]; then
  if inside 'command -v barkd' >/dev/null; then
    out="$(inside '
      set -u
      D=/data/lockdown-check
      rm -rf "$D"; mkdir -p "$D"
      barkd --datadir "$D" --host 127.0.0.1 --port 3999 --no-ui >/tmp/barkd-lockdown.log 2>&1 &
      pid=$!
      i=0
      while [ $i -lt 40 ]; do
        [ -f "$D/auth_token" ] && curl -sS -o /dev/null http://127.0.0.1:3999/ && break
        i=$((i + 1)); sleep 1
      done
      code=$(curl -sS -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $(cat "$D/auth_token" 2>/dev/null)" http://127.0.0.1:3999/api-docs/openapi.json)
      kill "$pid" 2>/dev/null
      wait "$pid" 2>/dev/null
      rm -rf "$D"
      echo "openapi=$code"
      tail -n 5 /tmp/barkd-lockdown.log
      rm -f /tmp/barkd-lockdown.log
    ')"
    case "$out" in
      *openapi=200*) ok "the wallet program (barkd) starts and answers under the same limits" ;;
      *) bad "the wallet program did not start under the limits: $out" ;;
    esac
  else
    echo "  (this image has no barkd; skipped the wallet program check)"
  fi
fi

echo
if [ "$fails" -eq 0 ]; then
  echo "Locked down: all checks passed."
  exit 0
fi
echo "$fails check(s) failed." >&2
exit 1
