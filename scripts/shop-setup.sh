#!/usr/bin/env bash
# EXPERIMENTAL IDEA, kept for later: not part of the supported recipe and never
# run on a real droplet (CI runs it on a clean Ubuntu 24.04 only).
#
# Set up a WordPress + WooCommerce shop on a fresh Ubuntu 24.04 server, with
# the Easy Bitcoin Payments plugin installed (not yet connected).
#
# This runs on the SHOP server, never on the payment server: the two are kept
# on separate servers so a hacked shop cannot reach the payment server.
# Started by deploy/shop-cloud-init.yaml; safe to run again (it skips what is
# already done). Guide: docs/SHOP_ON_DIGITALOCEAN.md
#
# What it installs, all from Ubuntu's own packages except where noted:
#   Caddy (web server with automatic https; from Caddy's official apt repository)
#   PHP 8.3 (FPM), MariaDB, WordPress and WooCommerce (via WP-CLI, from wordpress.org)
#   the EBP plugin, only after its release signature checks out (pinned cosign)
#   a firewall (ports 22, 80, 443), fail2ban and automatic security updates
#
# Settings (environment variables, all optional):
#   SHOP_DOMAIN     the shop's name, for example myshop.com. Empty: a free
#                   <ip>.sslip.io name, good for trying it out. Point the name
#                   at this server BEFORE running, or https cannot start.
#   SHOP_TITLE      the shop's name shown on the site (default "My shop")
#   SHOP_EMAIL      the admin email (default: none set, change it in WordPress)
#   SHOP_CURRENCY   WooCommerce currency code (default USD)
#   EBP_PLUGIN_VERSION  release to take the plugin from (default: newest)
# For tests only: SHOP_SKIP_FIREWALL=1, SHOP_CADDY_LOCAL_CERTS=1,
#   SHOP_CURL_OPTS=-k, SHOP_INFO_FILE, SHOP_SKIP_EBP_PLUGIN=1
set -euo pipefail

SHOP_DOMAIN="${SHOP_DOMAIN:-}"
SHOP_TITLE="${SHOP_TITLE:-My shop}"
SHOP_EMAIL="${SHOP_EMAIL:-}"
SHOP_CURRENCY="${SHOP_CURRENCY:-USD}"
EBP_PLUGIN_VERSION="${EBP_PLUGIN_VERSION:-latest}"
WEB_ROOT="/var/www/shop"
INFO_FILE="${SHOP_INFO_FILE:-/root/shop-info.txt}"
PHPV="8.3"
REPO="reya01/easy_bitcoin_payments"
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CURL=(curl -fsSL --retry 3 --connect-timeout 10 --max-time 300)
read -r -a CHECK_OPTS <<< "${SHOP_CURL_OPTS:-}"

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
# Run from the web folder: WP-CLI starts copies of itself, which fails when the
# current folder is one www-data cannot open.
wp() { (cd "$WEB_ROOT" && sudo -u www-data -- /usr/local/bin/wp --path="$WEB_ROOT" "$@"); }
secret() { openssl rand -base64 48 | tr -dc 'A-Za-z0-9' | head -c "$1"; }

[ "$(id -u)" = 0 ] || die "run as root (sudo)"
if ! grep -q 'VERSION_ID="24.04"' /etc/os-release 2>/dev/null; then
  say "warning: this was written and tested for Ubuntu 24.04; carrying on"
fi

# ---------------------------------------------------------------- the name
if [ -z "$SHOP_DOMAIN" ]; then
  # DigitalOcean's own metadata service first (stays inside DigitalOcean),
  # then the same public lookups start.sh uses.
  ip="$(curl -fsS -m 5 http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address 2>/dev/null \
     || curl -fsS -m 10 https://api.ipify.org 2>/dev/null \
     || curl -fsS -m 10 https://ifconfig.me 2>/dev/null || true)"
  [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "could not find this server's public IP; set SHOP_DOMAIN"
  SHOP_DOMAIN="${ip//./-}.sslip.io"
fi
SHOP_DOMAIN="${SHOP_DOMAIN,,}"
[[ "$SHOP_DOMAIN" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$|^localhost$ ]] \
  || die "SHOP_DOMAIN '$SHOP_DOMAIN' is not a plain domain name (like myshop.com, no https:// and no slashes)"
SITE_URL="https://${SHOP_DOMAIN}"
say "shop address: ${SITE_URL}"

# ---------------------------------------------------------------- packages
export DEBIAN_FRONTEND=noninteractive
say "installing packages (a few minutes)"
apt-get update -qq
apt-get install -y -qq ca-certificates curl gnupg debian-keyring debian-archive-keyring apt-transport-https \
  unzip openssl sudo cron ufw fail2ban unattended-upgrades mariadb-server \
  "php${PHPV}-fpm" "php${PHPV}-mysql" "php${PHPV}-curl" "php${PHPV}-gd" "php${PHPV}-intl" \
  "php${PHPV}-mbstring" "php${PHPV}-xml" "php${PHPV}-zip" "php${PHPV}-bcmath" >/dev/null

if ! command -v caddy >/dev/null 2>&1; then
  say "installing Caddy from its official repository"
  "${CURL[@]}" 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  "${CURL[@]}" 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' > /etc/apt/sources.list.d/caddy-stable.list
  chmod o+r /usr/share/keyrings/caddy-stable-archive-keyring.gpg /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -qq
  apt-get install -y -qq caddy >/dev/null
fi

# Small servers: a swap file keeps MariaDB and PHP from running out of memory.
mem_mb="$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)"
if [ "$mem_mb" -lt 1900 ] && ! swapon --show | grep -q . && [ ! -e /swapfile ]; then
  say "adding a 1 GB swap file (this server has ${mem_mb} MB of memory)"
  fallocate -l 1G /swapfile && chmod 600 /swapfile && mkswap -q /swapfile && swapon /swapfile
  echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi

# ---------------------------------------------------------------- PHP
cat > "/etc/php/${PHPV}/fpm/conf.d/90-shop.ini" <<INI
memory_limit = 256M
upload_max_filesize = 64M
post_max_size = 64M
max_execution_time = 120
expose_php = Off
INI
systemctl enable --now "php${PHPV}-fpm" mariadb >/dev/null 2>&1
systemctl restart "php${PHPV}-fpm"

# ---------------------------------------------------------------- WP-CLI
if [ ! -x /usr/local/bin/wp ]; then
  say "installing WP-CLI (checked against its published SHA-512)"
  tmp="$(mktemp -d)"
  base="https://raw.githubusercontent.com/wp-cli/builds/gh-pages/phar"
  "${CURL[@]}" -o "$tmp/wp-cli.phar" "$base/wp-cli.phar"
  "${CURL[@]}" -o "$tmp/wp-cli.phar.sha512" "$base/wp-cli.phar.sha512"
  echo "$(cut -c1-128 "$tmp/wp-cli.phar.sha512")  $tmp/wp-cli.phar" | sha512sum -c --quiet \
    || die "WP-CLI download did not match its checksum"
  install -m 0755 "$tmp/wp-cli.phar" /usr/local/bin/wp
  rm -rf "$tmp"
fi
mkdir -p /var/www/.wp-cli/cache && chown -R www-data:www-data /var/www/.wp-cli

# ---------------------------------------------------------------- WordPress
mkdir -p "$WEB_ROOT" && chown www-data:www-data "$WEB_ROOT"
if [ ! -f "$WEB_ROOT/wp-config.php" ]; then
  say "creating the database"
  db_pass="$(secret 32)"
  mysql --protocol=socket -uroot <<SQL
CREATE DATABASE IF NOT EXISTS shop CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER IF NOT EXISTS 'shop'@'localhost' IDENTIFIED BY '${db_pass}';
ALTER USER 'shop'@'localhost' IDENTIFIED BY '${db_pass}';
GRANT ALL PRIVILEGES ON shop.* TO 'shop'@'localhost';
FLUSH PRIVILEGES;
SQL
  say "downloading WordPress"
  [ -f "$WEB_ROOT/wp-load.php" ] || wp core download --quiet
  wp config create --dbname=shop --dbuser=shop --dbpass="$db_pass" --dbhost=localhost --quiet \
    --extra-php <<'PHP'
define( 'DISALLOW_FILE_EDIT', true );
define( 'DISABLE_WP_CRON', true );
define( 'WP_MEMORY_LIMIT', '256M' );
define( 'FORCE_SSL_ADMIN', true );
PHP
  chmod 640 "$WEB_ROOT/wp-config.php"
fi

admin_user="shopowner"
if ! wp core is-installed 2>/dev/null; then
  say "installing WordPress"
  admin_pass="$(secret 24)"
  email="${SHOP_EMAIL:-owner@example.com}"
  wp core install --url="$SITE_URL" --title="$SHOP_TITLE" --admin_user="$admin_user" \
    --admin_password="$admin_pass" --admin_email="$email" --skip-email --quiet
  wp rewrite structure '/%postname%/' --quiet
  wp plugin delete hello akismet --quiet 2>/dev/null || true
  umask 077
  cat > "$INFO_FILE" <<INFO
Your shop is set up.

  Shop:            ${SITE_URL}
  WordPress admin: ${SITE_URL}/wp-admin
  Username:        ${admin_user}
  Password:        ${admin_pass}

Next:
  1. Sign in, then change this password (Users, Profile) to one from your
     password manager. Turn on two-step sign-in (docs/KEEPING_WORDPRESS_SAFE.md).
  2. Set your real email address in Settings, General.
  3. Delete this file once the password is saved:  rm ${INFO_FILE}
  4. Connect the Bitcoin plugin to your payment server: docs/WOOCOMMERCE.md
INFO
  umask 022
fi

# ---------------------------------------------------------------- WooCommerce
if ! wp plugin is-active woocommerce 2>/dev/null; then
  say "installing WooCommerce"
  wp plugin install woocommerce --activate --quiet
  wp option update woocommerce_currency "$SHOP_CURRENCY" --quiet
fi
wp plugin auto-updates enable woocommerce --quiet 2>/dev/null || true

# ---------------------------------------------------------------- EBP plugin
if [ "${SHOP_SKIP_EBP_PLUGIN:-0}" != 1 ] && ! wp plugin is-installed ebp-bitcoin 2>/dev/null; then
  if [ "$EBP_PLUGIN_VERSION" = latest ]; then
    json="$("${CURL[@]}" "https://api.github.com/repos/${REPO}/releases?per_page=1" 2>/dev/null)" || json=""
    TAG="$(printf '%s' "$json" | grep -o '"tag_name": *"[^"]*"' | head -n 1 | sed 's/.*"\(v[^"]*\)"$/\1/' || true)"
    if [[ ! "$TAG" =~ ^v[0-9] ]]; then
      # GitHub's API limits unsigned requests per address; its releases feed does not.
      feed="$("${CURL[@]}" "https://github.com/${REPO}/releases.atom" 2>/dev/null)" || feed=""
      TAG="$(printf '%s' "$feed" | grep -o "releases/tag/v[^\"<]*" | head -n 1 | sed 's#releases/tag/##' || true)"
    fi
  else
    TAG="v${EBP_PLUGIN_VERSION#v}"
  fi
  [[ "$TAG" =~ ^v[0-9] ]] || die "could not work out the newest EBP release; set EBP_PLUGIN_VERSION"
  say "installing the Easy Bitcoin Payments plugin ${TAG} (signature checked first)"
  tmp="$(mktemp -d)"; chmod 755 "$tmp"
  base="https://github.com/${REPO}/releases/download/${TAG}"
  "${CURL[@]}" -o "$tmp/ebp-bitcoin-woocommerce.zip" "$base/ebp-bitcoin-woocommerce.zip"
  "${CURL[@]}" -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
  "${CURL[@]}" -o "$tmp/SHA256SUMS.sigstore.json" "$base/SHA256SUMS.sigstore.json"
  [ -x "$REPO_DIR/bin/cosign" ] && "$REPO_DIR/scripts/get-cosign.sh" --verify-local >/dev/null 2>&1 \
    || "$REPO_DIR/scripts/get-cosign.sh" >/dev/null
  "$REPO_DIR/bin/cosign" verify-blob --bundle "$tmp/SHA256SUMS.sigstore.json" \
    --certificate-identity "https://github.com/${REPO}/.github/workflows/release.yml@refs/tags/${TAG}" \
    --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
    "$tmp/SHA256SUMS" >/dev/null 2>&1 \
    || die "the plugin's checksum file was NOT signed by this project's release workflow for ${TAG}; not installed"
  grep -F " ebp-bitcoin-woocommerce.zip" "$tmp/SHA256SUMS" | sed "s#  #  $tmp/#" | sha256sum -c --quiet \
    || die "the plugin zip does not match the signed checksum; not installed"
  wp plugin install "$tmp/ebp-bitcoin-woocommerce.zip" --activate --quiet
  rm -rf "$tmp"
fi

# ---------------------------------------------------------------- scheduled jobs
# A real timer instead of WordPress's visit-driven one, so the plugin's hourly
# check runs even when nobody visits the shop.
echo "*/5 * * * * www-data /usr/local/bin/wp --path=${WEB_ROOT} cron event run --due-now --quiet >/dev/null 2>&1" \
  > /etc/cron.d/shop-wp-cron
chmod 644 /etc/cron.d/shop-wp-cron

# ---------------------------------------------------------------- web server
global=""
[ "${SHOP_CADDY_LOCAL_CERTS:-0}" = 1 ] && global=$'{\n\tlocal_certs\n}\n'
cat > /etc/caddy/Caddyfile <<CADDY
${global}
# Written by scripts/shop-setup.sh. Running it again rewrites this file.
${SHOP_DOMAIN} {
	root * ${WEB_ROOT}
	encode zstd gzip

	# Not needed by a shop, and often attacked.
	@blocked {
		path /xmlrpc.php /wp-config.php /readme.html /license.txt /.* /wp-content/uploads/*.php
	}
	respond @blocked 404

	header {
		Strict-Transport-Security "max-age=31536000"
		X-Content-Type-Options "nosniff"
		Referrer-Policy "strict-origin-when-cross-origin"
		X-Frame-Options "SAMEORIGIN"
		-Server
	}

	php_fastcgi unix//run/php/php${PHPV}-fpm.sock
	file_server
}
CADDY
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1 \
  || { caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile; die "web server settings are not valid"; }
systemctl enable caddy >/dev/null 2>&1
systemctl restart caddy

# ---------------------------------------------------------------- protection
if [ "${SHOP_SKIP_FIREWALL:-0}" != 1 ]; then
  ufw allow OpenSSH >/dev/null; ufw allow 80/tcp >/dev/null; ufw allow 443/tcp >/dev/null
  ufw --force enable >/dev/null
  systemctl enable --now fail2ban >/dev/null 2>&1 || true
  # Ubuntu installs security updates by itself once a day; make sure it is on.
  printf 'APT::Periodic::Update-Package-Lists "1";\nAPT::Periodic::Unattended-Upgrade "1";\n' \
    > /etc/apt/apt.conf.d/20auto-upgrades
fi

# ---------------------------------------------------------------- check
say "waiting for the padlock address to answer"
ok=0
for _ in $(seq 1 30); do
  if curl -fsS "${CHECK_OPTS[@]}" -m 10 -o /dev/null "${SITE_URL}/wp-login.php"; then ok=1; break; fi
  sleep 5
done
if [ "$ok" = 1 ]; then
  say "the shop answers at ${SITE_URL}"
else
  say "warning: ${SITE_URL} is not answering yet. If you used your own name, check it"
  say "points at this server, then run this script again. Nothing is lost."
fi

if [ -f "$INFO_FILE" ]; then
  if [ "$INFO_FILE" = /root/shop-info.txt ]; then
    printf '\nYour shop: %s  (sign-in details: sudo cat %s)\n\n' "$SITE_URL" "$INFO_FILE" > /etc/motd
  fi
  say "done. Sign-in details: sudo cat ${INFO_FILE}"
else
  say "done (already set up before; sign-in details were shown the first time)."
fi
