#!/usr/bin/env bash
# Checks scripts/address-helper.sh with a fake docker program: no real server, no
# network. The hard rules: a request is read strictly and used once, only plain
# host names get into .env, and every outcome leaves a clear status for EBP.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
fail() { echo "FAIL: $1" >&2; exit 1; }
ID="0123456789abcdef0123"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/repo/scripts" "$work/repo/updates/inbox" "$work/repo/updates/status" "$work/bin"
cp "$here/scripts/address-helper.sh" "$work/repo/scripts/"
cat > "$work/bin/docker" <<F
#!/usr/bin/env bash
echo "\$*" >> "$work/docker.log"
[ "\${FAKE_DOCKER_FAILS:-}" = 1 ] && exit 1
exit 0
F
chmod +x "$work/bin/docker" "$work/repo/scripts/address-helper.sh"
cd "$work/repo"
printf 'EBP_IMAGE=ghcr.io/x@sha256:abc\nEBP_DOMAIN=203-0-113-7.sslip.io\nEBP_PUBLIC_BASE_URL=https://203-0-113-7.sslip.io\n' > .env
chmod 0600 .env
run() { PATH="$work/bin:$PATH" ./scripts/address-helper.sh; }
req() { printf '%s' "$1" > updates/inbox/address-request.json; }
status() { cat updates/status/address-status.json; }

# Nothing asked: it only says it is installed.
run
grep -q '"helper_version":1' updates/status/address-helper.json || fail "helper did not say it is installed"
[ ! -e updates/status/address-status.json ] || fail "a status without a request"

# Serve the own domain next to the backup.
req "{\"action\":\"address\",\"main\":\"203-0-113-7.sslip.io\",\"others\":[\"pay.myshop.com\"],\"id\":\"$ID\"}"
run
status | grep -q "\"state\":\"done\".*\"id\":\"$ID\"" || fail "serve not done: $(status)"
grep -qx 'EBP_SITE_NAMES=203-0-113-7.sslip.io, pay.myshop.com' .env || fail "site names not written"
grep -qx 'EBP_PUBLIC_ALIASES=https://pay.myshop.com' .env || fail "aliases not written"
grep -qx 'EBP_PUBLIC_BASE_URL=https://203-0-113-7.sslip.io' .env || fail "main address changed too early"
grep -qx 'EBP_IMAGE=ghcr.io/x@sha256:abc' .env || fail "other settings lost"
[ "$(stat -c %a .env)" = 600 ] || fail ".env permissions changed"
grep -q 'compose up -d --no-deps caddy ebp' "$work/docker.log" || fail "web server and EBP not restarted"
[ ! -e updates/inbox/address-request.json ] || fail "request not used up"

# Make it the main address.
req "{\"action\":\"address\",\"main\":\"pay.myshop.com\",\"others\":[\"203-0-113-7.sslip.io\"],\"id\":\"$ID\"}"
run
grep -qx 'EBP_DOMAIN=pay.myshop.com' .env || fail "main domain not switched"
grep -qx 'EBP_PUBLIC_BASE_URL=https://pay.myshop.com' .env || fail "main address not switched"
grep -qx 'EBP_PUBLIC_ALIASES=https://203-0-113-7.sslip.io' .env || fail "backup not kept"
[ "$(grep -c '^EBP_DOMAIN=' .env)" = 1 ] || fail "EBP_DOMAIN duplicated"

# Give up the backup.
req "{\"action\":\"address\",\"main\":\"pay.myshop.com\",\"others\":[],\"id\":\"$ID\"}"
run
grep -qx 'EBP_SITE_NAMES=pay.myshop.com' .env || fail "backup not given up"
grep -qx 'EBP_PUBLIC_ALIASES=' .env || fail "alias not cleared"

# Refused requests change nothing.
cp .env "$work/env.before"
for bad in \
  '{"action":"address","main":"pay.myshop.com\nEBP_IMAGE=evil","others":[],"id":"0123456789abcdef0123"}' \
  '{"action":"address","main":"pay.myshop.com; rm -rf /","others":[],"id":"0123456789abcdef0123"}' \
  '{"action":"address","main":"203.0.113.7","others":[],"id":"0123456789abcdef0123"}' \
  '{"action":"address","main":"PAY.myshop.com","others":[],"id":"0123456789abcdef0123"}' \
  '{"action":"address","main":"pay.myshop.com","others":["a.com","b.com","c.com"],"id":"0123456789abcdef0123"}' \
  '{"action":"address","main":"pay.myshop.com","others":["pay.myshop.com"],"id":"0123456789abcdef0123"}' \
  '{"action":"apply","main":"pay.myshop.com","others":[],"id":"0123456789abcdef0123"}' \
  '{"action":"address","main":"pay.myshop.com","others":[],"id":"short"}' \
  'not json'; do
  req "$bad"
  run
  status | grep -q '"state":"failed","code":"bad_request"' || fail "not refused: $bad"
  cmp -s .env "$work/env.before" || fail "a refused request changed .env: $bad"
done

# A symlink in place of the request is not followed.
ln -s /etc/passwd updates/inbox/address-request.json
run
status | grep -q 'bad_request' || fail "symlink request not refused"
[ ! -e updates/inbox/address-request.json ] || fail "symlink request left behind"

# A failed restart is reported.
req "{\"action\":\"address\",\"main\":\"pay.myshop.com\",\"others\":[],\"id\":\"$ID\"}"
FAKE_DOCKER_FAILS=1 run
status | grep -q '"state":"failed","code":"restart_failed"' || fail "failed restart not reported"
echo "address helper tests passed"
