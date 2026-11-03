#!/usr/bin/env bash
# ebp backup — consistent snapshots of the payments stack, safe to run while
# the server is live (SQLite online-backup API; read-only volume mounts).
#
# Usage:   bash scripts/backup.sh [BACKUP_DIR]     # default ~/ebp-backups
# Run:     ON THE VPS, as a user in the `docker` group (or with sudo).
# Schedule (optional): crontab entry, e.g. daily 03:15 UTC —
#   15 3 * * * bash ~/easy_bitcoin_payments/scripts/backup.sh >> ~/ebp-backups/backup.log 2>&1
#
# What is backed up and what it is worth:
#   1. ebp.sqlite3   — invoices, stores, API keys, settings. Recoverable
#                      history; losing it means losing paid/unpaid states.
#   2. barkd datadir — the WALLET ITSELF (keys derived from the seed). This
#                      is the crown jewel: with it + the recovery words you
#                      can restore the wallet anywhere.
#   The recovery words on paper remain the ONLY human-verifiable backup of
#   the wallet. This script is the operational backup layer on top.
#
# Keeps the last 14 snapshots of each kind. Snapshots containing wallet
# material are chmod 600 — treat the whole backup dir as a secret.

set -euo pipefail

DEST="${1:-$HOME/ebp-backups}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
KEEP=14
mkdir -p "$DEST"

# Resolve the compose volumes from the RUNNING containers; fall back to the
# default compose project name when the stack is stopped.
ebp_vol="$(docker inspect easy_bitcoin_payments-ebp-1 \
  --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Name}}{{end}}{{end}}' 2>/dev/null || true)"
barkd_vol="$(docker inspect easy_bitcoin_payments-barkd-1 \
  --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Name}}{{end}}{{end}}' 2>/dev/null || true)"
ebp_vol="${ebp_vol:-easy_bitcoin_payments_ebp-data}"
barkd_vol="${barkd_vol:-easy_bitcoin_payments_barkd-data}"
for vol in "$ebp_vol" "$barkd_vol"; do
  docker volume inspect "$vol" >/dev/null 2>&1 \
    || { echo "ERROR: volume '$vol' not found — is the stack deployed with this compose project?" >&2; exit 1; }
done

echo "== 1/3 ebp database: online backup + integrity check =="
docker run --rm \
  -v "$ebp_vol":/data:ro -v "$DEST":/backup \
  alpine:3.20 sh -c \
  "apk add -q sqlite >/dev/null \
   && sqlite3 /data/ebp.sqlite3 \".backup '/backup/ebp-$STAMP.sqlite3'\" \
   && echo -n 'integrity: ' \
   && sqlite3 \"/backup/ebp-$STAMP.sqlite3\" 'PRAGMA integrity_check;'"
test "$(docker run --rm -v "$DEST":/backup alpine:3.20 \
  sh -c "sqlite3 \"/backup/ebp-$STAMP.sqlite3\" 'PRAGMA integrity_check;'")" = "ok" \
  || { echo "ERROR: integrity check failed — snapshot NOT trusted" >&2; exit 1; }

echo "== 2/3 barkd wallet datadir (CONTAINS WALLET SECRETS) =="
docker run --rm \
  -v "$barkd_vol":/data:ro -v "$DEST":/backup \
  alpine:3.20 sh -c "tar czf \"/backup/barkd-data-$STAMP.tar.gz\" -C /data ."
chmod 600 "$DEST/barkd-data-$STAMP.tar.gz"
chmod 600 "$DEST/ebp-$STAMP.sqlite3" 2>/dev/null || true

echo "== 3/3 retention (keep last $KEEP of each) =="
ls -1t "$DEST"/ebp-*.sqlite3      2>/dev/null | tail -n +$((KEEP + 1)) | xargs -r rm -f
ls -1t "$DEST"/barkd-data-*.tar.gz 2>/dev/null | tail -n +$((KEEP + 1)) | xargs -r rm -f
chown "$(id -u):$(id -g)" "$DEST"/ebp-"$STAMP".sqlite3 "$DEST"/barkd-data-"$STAMP".tar.gz 2>/dev/null || true

echo "OK — snapshot $STAMP in $DEST:"
ls -lh "$DEST" | tail -4
echo
echo "Reminder: the paper recovery words are the only human-verifiable wallet backup."
echo "Copy this directory OFF the VPS (scp/S3) to make it survive instance loss."
