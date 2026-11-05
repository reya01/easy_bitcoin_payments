#!/usr/bin/env bash
# Start (or restart) the Easy Bitcoin Payments stack and print the one-time
# SETUP CODE in this script's own final message, so a beginner never has to
# dig through logs. Run it from the folder that holds docker-compose.yml:
#
#   ./scripts/start.sh
#
# Any extra arguments are passed to `docker compose up -d`.
set -euo pipefail

cd "$(dirname "$0")/.."

docker compose up -d "$@"

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
  echo "  The server is running, but no setup code was printed."
  echo "  That usually means setup is already finished."
  echo "  If you expected a code, wait a minute and run this"
  echo "  script again, or run:  docker compose logs ebp"
fi
echo "=================================================="
