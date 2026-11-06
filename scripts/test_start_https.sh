#!/usr/bin/env bash
# Checks what ./scripts/start.sh writes for --https / --domain, using fake
# `docker` and `curl` programs (no real server, no network needed).
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/scripts" "$work/bin"
cp "$here/scripts/start.sh" "$work/scripts/"
cat > "$work/bin/docker" <<'F'
#!/usr/bin/env bash
if [ "$1 $2" = "compose logs" ]; then echo "SETUP CODE: ABCD-1234"; fi
exit 0
F
cat > "$work/bin/curl" <<'F'
#!/usr/bin/env bash
case "$*" in
  *api.ipify.org*) echo "203.0.113.7" ;;
esac
exit 0
F
sed -i 's/seq 1 45/seq 1 1/; s/sleep 4/true/' "$work/scripts/start.sh"
chmod +x "$work/bin/"* "$work/scripts/start.sh"
fail() { echo "FAIL: $1" >&2; exit 1; }

cd "$work"
PATH="$work/bin:$PATH" ./scripts/start.sh --https >/dev/null
grep -qx 'EBP_DOMAIN=203-0-113-7.sslip.io' .env || fail "sslip domain not written"
grep -qx 'EBP_PUBLIC_BASE_URL=https://203-0-113-7.sslip.io' .env || fail "base url not written"
grep -qx 'COMPOSE_PROFILES=https' .env || fail "profile not written"
grep -qx 'EBP_TRUST_PROXY_HEADERS=true' .env || fail "proxy trust not written"

PATH="$work/bin:$PATH" ./scripts/start.sh --domain pay.myshop.com >/dev/null
grep -qx 'EBP_DOMAIN=pay.myshop.com' .env || fail "own domain not written"
[ "$(grep -c '^EBP_DOMAIN=' .env)" = 1 ] || fail "EBP_DOMAIN duplicated"

out="$(PATH="$work/bin:$PATH" ./scripts/start.sh --domain 2>&1 || true)"
printf '%s' "$out" | grep -q 'needs a name' || fail "--domain without a name not explained"
echo "start.sh https tests passed"
