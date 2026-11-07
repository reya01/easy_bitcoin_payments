#!/usr/bin/env bash
# Start (or restart) the Easy Bitcoin Payments stack and print the one-time
# SETUP CODE in this script's own final message, so a beginner never has to
# dig through logs. Run it from the folder that holds docker-compose.yml:
#
#   ./scripts/start.sh
#
# Automatic https (a padlock address customers can use), one word:
#
#   ./scripts/start.sh --https                 # free address made from your server's IP
#   ./scripts/start.sh --domain pay.myshop.com # your own domain name
#
# Your server must be reachable from the internet on ports 80 and 443.
# The choice is remembered (in a file called .env), so later runs of plain
# ./scripts/start.sh keep https on.
#
# Any other arguments are passed to `docker compose up -d`.
set -euo pipefail

cd "$(dirname "$0")/.."

# Test hook only: extra curl flags for the https wait (CI uses -k because a
# local test certificate is not publicly trusted). Empty for everyone else.
read -r -a curl_extra <<< "${EBP_CURL_OPTS:-}"

want_https=""
domain=""
passthru=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --https) want_https=1 ;;
    --domain)
      shift
      domain="${1:-}"
      if [ -z "$domain" ]; then
        echo "error: --domain needs a name, e.g. --domain pay.myshop.com" >&2
        exit 1
      fi
      want_https=1
      ;;
    *) passthru+=("$1") ;;
  esac
  shift
done

# Write or replace KEY=VALUE in .env (docker compose reads it automatically).
set_env() {
  touch .env
  grep -v "^$1=" .env > .env.new || true
  echo "$1=$2" >> .env.new
  mv .env.new .env
}

if [ -n "$want_https" ]; then
  if [ -z "$domain" ]; then
    ip="$(curl -fsS -m 10 https://api.ipify.org 2>/dev/null || curl -fsS -m 10 https://ifconfig.me 2>/dev/null || true)"
    if ! printf '%s' "$ip" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
      echo "Could not work out this server's public address." >&2
      echo "Use your own domain instead:  ./scripts/start.sh --domain pay.myshop.com" >&2
      exit 1
    fi
    # sslip.io turns 203-0-113-7.sslip.io into 203.0.113.7: a free name with
    # a real certificate, no domain purchase and no DNS settings needed.
    domain="$(printf '%s' "$ip" | tr . -).sslip.io"
  fi
  set_env EBP_DOMAIN "$domain"
  set_env EBP_PUBLIC_BASE_URL "https://$domain"
  set_env EBP_TRUST_PROXY_HEADERS true
  set_env COMPOSE_PROFILES https
  export EBP_DOMAIN="$domain" EBP_PUBLIC_BASE_URL="https://$domain" EBP_TRUST_PROXY_HEADERS=true COMPOSE_PROFILES=https
  echo "==> https is ON for https://$domain"
  echo "    (your server must accept visitors on ports 80 and 443)"
fi

# Optional prebuilt image (EBP_IMAGE=ghcr.io/...@sha256:...). If the pull
# fails we say so and fall back to building from source.
if [ -n "${EBP_IMAGE:-}" ]; then
  docker compose pull ebp || echo "(could not pull $EBP_IMAGE - building from source instead)"
fi

docker compose up -d ${passthru[@]+"${passthru[@]}"}

echo "==> waiting for the server to start (up to 2 minutes)"
code=""
for _ in $(seq 1 60); do
  code="$(docker compose logs --no-color ebp 2>/dev/null \
    | grep -o 'SETUP CODE: *[A-Z0-9-]*' | tail -n 1 | sed 's/SETUP CODE: *//' || true)"
  if [ -n "$code" ]; then
    break
  fi
  sleep 2
done

base="${EBP_PUBLIC_BASE_URL:-http://localhost:8080}"
if [ "${COMPOSE_PROFILES:-}" = "https" ] || grep -q '^COMPOSE_PROFILES=https' .env 2>/dev/null; then
  base="$(grep '^EBP_PUBLIC_BASE_URL=' .env | tail -n 1 | cut -d= -f2-)"
  echo "==> waiting for the https padlock (the first time can take a minute)"
  https_ok=""
  for _ in $(seq 1 45); do
    if curl -fsS ${curl_extra[@]+"${curl_extra[@]}"} -m 5 -o /dev/null "$base/api/v1/health" 2>/dev/null; then
      https_ok=1
      break
    fi
    sleep 4
  done
  if [ -z "$https_ok" ]; then
    echo
    echo "  The padlock address is not answering yet. Usual causes:"
    echo "  - ports 80 and 443 are closed in your hosting provider's firewall"
    echo "  - (own domain only) the domain does not point at this server yet"
    echo "  Fix that, then run this script again. Nothing is lost."
  fi
fi
echo
echo "=================================================="
if [ -n "$code" ]; then
  echo "  The server is running."
  echo
  echo "  1. Open this page in your browser:"
  echo "       ${base}/setup"
  echo
  echo "  2. When it asks for your SETUP CODE, type:"
  echo
  echo "       ${code}"
  echo
  echo "  (Capitals, dashes and spaces do not matter. The code"
  echo "   changes if the server restarts before setup is done."
  echo "   Run this script again to see the new one.)"
else
  echo "  No setup code yet."
  echo "  - If this is the FIRST start, the server may still be starting."
  echo "    Wait a minute and run this script again."
  echo "  - If you already finished setup, there is no code any more:"
  echo "    sign in at ${base}/admin with your admin password."
  echo "  - Still stuck?  docker compose logs ebp"
fi
echo "=================================================="
