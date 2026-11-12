#!/usr/bin/env bash
# Capture the REAL shapes of barkd's read-only answers during the signet
# drill, so they can be compared with what the parser expects.
#
# Run on the server, from the folder with docker-compose.yml (single-container
# install), as a user that can run docker:
#
#   ./scripts/drill_capture.sh            # writes drill-capture-<time>.txt here
#
# Safety:
#  * Only a fixed list of read-only GET requests is made (no mnemonic
#    endpoint, no wallet debug dump, nothing that changes state).
#  * The barkd token is read INSIDE the container and used there; it is never
#    printed and never written to the output file.
#  * Signet data (addresses, transaction ids) is not secret, but look the file
#    over before sharing it. It never contains recovery words or API keys.
set -euo pipefail
cd "$(dirname "$0")/.."

out="drill-capture-$(date -u +%Y%m%dT%H%M%SZ).txt"

docker compose exec -T ebp sh -s >"$out" <<'INNER'
set -u
DATADIR=/data/barkd
T="$(barkd --datadir "$DATADIR" secret show 2>/dev/null | tr -d '\r\n')"
if [ -z "$T" ]; then echo "no barkd token (is the wallet created yet?)"; exit 0; fi
B=http://127.0.0.1:3000
echo "# captured $(date -u +%FT%TZ)"
echo
echo "## barkd data folder (names and sizes only)"
find "$DATADIR" -type f -exec ls -l {} + 2>/dev/null | awk '{print $5, $9}'
echo
for path in \
  /ping \
  /api/v1/wallet/connected \
  /api/v1/wallet/balance \
  /api/v1/wallet/ark-info \
  /api/v1/wallet/vtxos \
  /api/v1/wallet/rounds \
  /api/v1/wallet/next-round \
  /api/v1/wallet/history \
  /api/v1/history \
  /api/v1/wallet/movements \
  /api/v1/onchain/balance \
  /api/v1/onchain/transactions \
  /api/v1/onchain/utxos \
  /api/v1/bitcoin/tip \
  /api/v1/exits/status/all \
  /api/v1/fees/onchain \
  /api/v1/boards/pending
do
  echo "## GET $path"
  curl -sS -m 20 -w '\n[http %{http_code}]\n' -H "Authorization: Bearer $T" "$B$path" || echo "[curl failed]"
  echo
done
echo "## EBP health"
curl -sS -m 10 http://127.0.0.1:8080/api/v1/health
echo
INNER

echo "Saved $out"
echo "Check it, then attach it to your drill report (it holds no recovery words, API keys or tokens)."
