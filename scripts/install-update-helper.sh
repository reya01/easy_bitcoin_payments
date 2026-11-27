#!/usr/bin/env bash
# Install (or remove) the EBP update helper, so updates can be started from the
# admin page's Home tab. Run once, as root, from the EBP folder:
#
#   sudo ./scripts/install-update-helper.sh             # install or refresh
#   sudo ./scripts/install-update-helper.sh --uninstall # remove it again
#
# What it does:
#   * makes updates/inbox (EBP may write a request there) and updates/status
#     (only root writes there; EBP reads it), see scripts/update-helper.sh;
#   * installs small systemd units that run the helpers when a request appears:
#     the update helper, and the address helper (moving to your own domain,
#     docs/OWN_DOMAIN.md), which works the same way;
#   * runs the helper once so the admin page can see it is installed.
# It never updates anything by itself. Updates still happen only when the owner
# presses Update on the Home tab, twice, after seeing the signature result.
set -euo pipefail

main() {
  cd "$(dirname "$0")/.."
  local dir; dir="$(pwd)"
  local units="${EBP_SYSTEMD_DIR:-/etc/systemd/system}"
  local ebp_uid="${EBP_CONTAINER_UID:-10001}"

  [ "$(id -u)" = 0 ] || { echo "Run this as root (for example: sudo $0)." >&2; exit 1; }
  case "$dir" in *[!A-Za-z0-9/._-]*) echo "The EBP folder path has unusual characters: $dir" >&2; exit 1 ;; esac

  if [ "${1:-}" = "--uninstall" ]; then
    systemctl disable --now ebp-update-helper.path ebp-address-helper.path 2>/dev/null || true
    rm -f "$units/ebp-update-helper.path" "$units/ebp-update-helper.service" \
      "$units/ebp-address-helper.path" "$units/ebp-address-helper.service"
    systemctl daemon-reload 2>/dev/null || true
    echo "The update helper was removed. Update from the Console with ./scripts/update.sh instead."
    exit 0
  fi

  command -v python3 >/dev/null || { echo "python3 is needed (it is on Ubuntu by default)." >&2; exit 1; }
  command -v flock >/dev/null || { echo "flock is needed (package util-linux)." >&2; exit 1; }

  install -d -m 0755 -o root -g root "$dir/updates" "$dir/updates/status"
  install -d -m 0770 -o "$ebp_uid" -g "$ebp_uid" "$dir/updates/inbox"

  for u in ebp-update-helper.path ebp-update-helper.service ebp-address-helper.path ebp-address-helper.service; do
    sed "s#/opt/ebp#$dir#g" "deploy/systemd/$u" > "$units/$u.new"
    chmod 0644 "$units/$u.new"
    mv -f "$units/$u.new" "$units/$u"
  done
  systemctl daemon-reload
  systemctl enable --now ebp-update-helper.path ebp-address-helper.path >/dev/null

  # Run once so the admin page can see the helpers are installed.
  ./scripts/update-helper.sh || true
  ./scripts/address-helper.sh || true

  echo "The update helper is installed. From now on you can update from the admin"
  echo "page: Home tab, Update card. You decide twice and see the signature first."
  echo "To remove it: sudo $0 --uninstall"
}

main "$@"
