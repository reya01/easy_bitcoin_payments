#!/usr/bin/env bash
# One-shot setup for a fresh Ubuntu server, run by cloud-init (see
# docs/START_HERE.md and deploy/cloud-init.yaml). It:
#   1. adds a little swap so building on a small server cannot run out of memory
#   2. starts Docker
#   3. runs ./scripts/start.sh --https (free padlock address)
#   4. copies the "open this page / type this code" box into
#      /root/EBP-START-HERE.txt AND the console login screen (/etc/issue)
#      and the after-login message (/etc/motd),
#      so a beginner sees it in the provider's web console without a terminal.
#
# Safe to run again: it only adds swap once and refreshes the box.
set -euo pipefail

EBP_DIR="${EBP_DIR:-/opt/ebp}"
START_CMD="${EBP_START_CMD:-}"
INFO_FILE="${EBP_INFO_FILE:-/root/EBP-START-HERE.txt}"
ISSUE_FILE="${EBP_ISSUE_FILE:-/etc/issue}"
MOTD_FILE="${EBP_MOTD_FILE:-/etc/motd}"
LOG_FILE="${EBP_LOG_FILE:-/var/log/ebp-start.log}"
SWAP_FILE="${EBP_SWAP_FILE:-/swapfile}"
SKIP_SYSTEM="${EBP_SKIP_SYSTEM:-}"

if [ -z "$SKIP_SYSTEM" ]; then
  if ! swapon --show | grep -q .; then
    fallocate -l 2G "$SWAP_FILE" 2>/dev/null || dd if=/dev/zero of="$SWAP_FILE" bs=1M count=2048 status=none
    chmod 600 "$SWAP_FILE"
    mkswap "$SWAP_FILE" >/dev/null
    swapon "$SWAP_FILE"
    grep -q "$SWAP_FILE" /etc/fstab || echo "$SWAP_FILE none swap sw 0 0" >> /etc/fstab
  fi
  systemctl enable --now docker
fi

cd "$EBP_DIR"
if [ -z "$START_CMD" ]; then
  if grep -q '^EBP_DOMAIN=' .env 2>/dev/null; then
    # A web address is already saved (your own domain, or the free one made
    # on the first run): keep it. Running with --https again could replace a
    # custom domain with the free name.
    START_CMD="./scripts/start.sh"
  else
    START_CMD="./scripts/start.sh --https"
  fi
fi
# shellcheck disable=SC2086 # START_CMD is a command line on purpose
$START_CMD 2>&1 | tee "$LOG_FILE" || true

# The box printed by start.sh sits between two lines of '='.
box="$(awk '/^=+$/ {n++; next} n==1 {print}' "$LOG_FILE" | sed 's/^  //')"
if [ -z "$box" ]; then
  box="The server is still starting. Wait two minutes, then run:  ebp-info"
fi

{
  echo "Easy Bitcoin Payments"
  echo "---------------------"
  echo "$box"
  echo
  echo "Lost this screen? Run:  ebp-info"
} > "$INFO_FILE"
chmod 600 "$INFO_FILE"

# Show it on the provider's web console login screen too.
if [ -w "$ISSUE_FILE" ] || [ ! -e "$ISSUE_FILE" ]; then
  {
    echo
    cat "$INFO_FILE"
    echo
  } > "$ISSUE_FILE"
fi

# And the message shown right after logging in (the browser Console and SSH).
if [ -w "$MOTD_FILE" ] || [ ! -e "$MOTD_FILE" ]; then
  {
    echo
    cat "$INFO_FILE"
    echo
  } > "$MOTD_FILE"
fi

# `ebp-info` re-runs start.sh (harmless) so the setup code is always current.
if [ -z "$SKIP_SYSTEM" ]; then
  cat > /usr/local/bin/ebp-info <<INNER
#!/usr/bin/env bash
EBP_DIR="$EBP_DIR" exec "$EBP_DIR/scripts/vps-setup.sh"
INNER
  chmod 755 /usr/local/bin/ebp-info
fi
cat "$INFO_FILE"
