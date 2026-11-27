# Technical overview

For developers. Shop owners: start with [START_HERE.md](../START_HERE.md).

Easy Bitcoin Payments (EBP) is a single Rust binary plus SQLite that sits between
your store and Bitcoin. No custodian, no middleman, no software fees. Your keys,
your coins, your server.

This repository is at **v0.3.0-rc3** (a release candidate, 21 November 2026) and remains
**EXPERIMENTAL**: signet (play money) by default; mainnet is opt-in (`EBP_MODE=live`) for
small test amounts only. It has been tried end to end with small real-money amounts (see
[MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md)) but is not for production. CI is the
source of truth for tests (it builds, runs `cargo test --all --all-features`, and runs the
browser tests in `tests/e2e/`, see [E2E_TESTING.md](E2E_TESTING.md)); test counts are not
written down here because they go stale at once.

The wallet layer talks to a [barkd](https://second.tech/docs/barkd_index.md) sidecar
(Second's reference Bark wallet daemon, **pinned to barkd 0.9.0**; what the move from 0.8 involved is in
[BARKD_0_9_0_UPGRADE.md](BARKD_0_9_0_UPGRADE.md) and [archive/BARK_0_9_REVIEW.md](archive/BARK_0_9_REVIEW.md)) over its local REST API:
Ark and Lightning receive, a polling payment watcher with a persisted cursor, a per-coin
expiry queue, and a settlement engine that sends the whole balance to your own wallet once
it reaches a minimum. **EBP takes no regular (on-chain) payments**; the only on-chain things
are the payout to your own on-chain address and the emergency withdrawal. The barkd
request and response shapes EBP relies on are pinned in
[`src/wallet/barkd/live_shapes.rs`](../../src/wallet/barkd/live_shapes.rs) with shape tests, and
`scripts/check_barkd_drift.sh` checks every endpoint against the pinned OpenAPI in CI.

### Documentation map

For shop owners: [START_HERE.md](../START_HERE.md), [WOOCOMMERCE.md](../WOOCOMMERCE.md),
[KEEPING_WORDPRESS_SAFE.md](../KEEPING_WORDPRESS_SAFE.md), [OTHER_HOSTS.md](../OTHER_HOSTS.md),
[RECIPE_BACKUP.md](../RECIPE_BACKUP.md), [RESTORE_DRILL.md](../RESTORE_DRILL.md),
[UPGRADING.md](../UPGRADING.md), [IF_SOMETHING_LOOKS_WRONG.md](../IF_SOMETHING_LOOKS_WRONG.md),
[KNOWN_ISSUES.md](../KNOWN_ISSUES.md), [SILENT_PAYMENTS.md](../SILENT_PAYMENTS.md).

For developers (this folder):

- [HANDOFF.md](HANDOFF.md): one-stop context for a new developer or agent. Start here.
- Where things stand: [MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md) (what is proven on real money and what is not),
  [BARKD_0_9_0_UPGRADE.md](BARKD_0_9_0_UPGRADE.md) (the recipe for moving to a new barkd).
- Release and testing: [RC_CHECKLIST.md](RC_CHECKLIST.md), [DROPLET_TEST_SHEET.md](DROPLET_TEST_SHEET.md) (the test-day sheet for a live server),
  [SIGNET_DRILL.md](SIGNET_DRILL.md), [E2E_TESTING.md](E2E_TESTING.md), [REPRODUCIBLE_BUILDS.md](REPRODUCIBLE_BUILDS.md).
- Security and wallet: [SHOP_COMPROMISE.md](SHOP_COMPROMISE.md) (what a hacked shop can and cannot do).
- Plans and design notes: [SHOPIFY.md](SHOPIFY.md), [BRANTA.md](BRANTA.md), [spec.md](spec.md) (the original spec).
- [archive/](archive/README.md): finished plans, reviews, drills and test notes, kept for history (the mainnet plan, the Bark audit and 0.9 review, the emergency-exit drill, the old checklists).
- [CONTRIBUTING.md](../../CONTRIBUTING.md), [ebp.example.toml](../../ebp.example.toml) (every config key), [LICENSE](../../LICENSE).

---

## What ships today

- **Rust server** — axum 0.8 + SQLite (rusqlite, bundled), single binary, MIT licensed, CI on main and pull requests.
- **Invoice API** — create / get / list / cancel, authenticated with per-store API keys (hashed at rest); the shop's key can do only what a shop needs (see [SHOP_COMPROMISE.md](SHOP_COMPROMISE.md)).
- **Hosted checkout page** — static, mobile-first, live status polling, plain-language states. Lightning is the default; one box switches to Ark. Dollar amounts come from one stored Bitcoin price.
- **Payment watcher** — polls barkd's wallet history with a persisted cursor; address matching, idempotent payment recording, per-coin expiry queue, late and unmatched payments listed for the owner.
- **Signed webhooks** — `X-EBP-Signature: t=<unix>,v1=<HMAC_SHA256>` with a replay window, a durable delivery queue with a Resend button, and an hourly shop-side check that finishes orders whose notice was lost.
- **Payouts** — the whole balance goes to the owner's own wallet once it reaches a minimum (hard floor per kind of address); a payout-address change waits a day; "Move my money now" by hand; coins that are renewing are never sent.
- **Emergency withdrawal** — opt-in, started by hand, with protected fee money (see "What stays on the server" below).
- **Admin page** at `/admin` (also `/dashboard`) — mobile-first: Home, payments, shop connection, alerts, settings. Sign-in is the admin password plus two-step sign-in (authenticator app, recovery codes) and passkeys (beta); protected actions need the password plus a code or a passkey approval.
- **Backup and restore** — one encrypted backup file plus the 12 recovery words; restore on a new server at setup.
- **Phone alerts** (ntfy) and a daily summary that never carries amounts or addresses.
- **Updates from the Home tab** — a root helper outside EBP checks the signature and image and updates only after the owner confirms; `scripts/update.sh` does the same from the Console.
- **WooCommerce plugin** (`integrations/woocommerce/`), built and tested against a real WordPress in CI.
- **Config** — env vars plus an optional TOML file, with sane defaults.
- **Deployment** — a pasted setup block on a DigitalOcean droplet (public IP, DNS, TLS), or Docker Compose on any VPS; the container runs read-only with no extra powers.

### The wallet layer: the barkd sidecar

Payments run through a **barkd sidecar** — Second's reference Bark wallet
daemon — on the internal network. barkd holds the keys (Custody Level 0: a
small "till" on the server that is sent whole to your own wallet once it reaches a
minimum; everything the server sends is already yours). The server authenticates with a bearer token:

```bash
# barkd generates the token on first start and stores it in its datadir.
export EBP_BARKD_TOKEN="$(barkd --datadir /path/to/bark secret show)"
```

The token is env-only (`EBP_BARKD_TOKEN`), lives in the deployment env / a
`.env` file next to the compose file, and is never logged. Rotate it with
`barkd secret refresh` + a restart of both daemons. `--expose-mnemonic` stays
off on the compose service; the setup wizard's one-time backup ceremony runs
a separate temporary barkd WITH the flag, reads `GET /wallet/mnemonic` exactly
once, then removes it — the words are never stored or logged by EBP.

Without a token the server boots on a stub wallet (deterministic
placeholder addresses, no payments observed) so development and tests never
need a running barkd.

---

## Quickstart

Requires Rust 1.82+ (pinned in `rust-toolchain.toml`).

```bash
# 1. Create a store. The API key is printed once — save it.
cargo run -- store create --name "My Shop" --webhook-url "https://shop.example.com/ebp"
cargo run -- store list
cargo run -- admin set-password         # forgot the admin-page password? reset it here
cargo run -- admin reset-2fa            # lost your phone and your backup codes? turns two-step sign-in off (and removes passkeys)
cargo run -- store rotate-key            # lost your API key? mints a new one, shown once

# 2. Start the server.
cargo run -- serve
```

The server listens on `http://localhost:8080`.

```bash
# 3. Create an invoice.
curl -sS -X POST http://localhost:8080/api/v1/invoices \
  -H "Authorization: Bearer ebp_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
        "order_id": "wc-1234",
        "fiat": { "currency": "EUR", "amount": "42.00" },
        "expiry_minutes": 20
      }'
```

The response contains `checkout_url`, `ark_address`, `lightning_invoice` and
`lightning_fee_estimate_sat` (when Lightning is on), `amount_sats`, the locked `rate`, and
`expires_at`. There is no on-chain address: EBP takes no regular Bitcoin payments. Open `checkout_url` on a
phone — that is the customer's page.

```bash
# 4. Read it back (the source of truth for your store).
curl -sS http://localhost:8080/api/v1/invoices/inv_... \
  -H "Authorization: Bearer ebp_sk_..."

# 5. List, filter, cancel.
curl -sS "http://localhost:8080/api/v1/invoices?status=pending&limit=20" \
  -H "Authorization: Bearer ebp_sk_..."
curl -sS -X POST http://localhost:8080/api/v1/invoices/inv_.../cancel \
  -H "Authorization: Bearer ebp_sk_..."

# 6. Health.
curl -sS http://localhost:8080/api/v1/health
```

### With the barkd sidecar

```bash
# 1. Run barkd (see https://second.tech/docs/barkd/install.md) and get its token.
barkd --datadir ~/.bark-ebp &
export EBP_BARKD_TOKEN="$(barkd --datadir ~/.bark-ebp secret show)"

# 2. Point ebp at it and start the server.
export EBP_BARKD_URL="http://127.0.0.1:3000/api/v1"
cargo run -- serve

# 3. Open http://localhost:8080/setup and let the wizard create the wallet.
#    The wizard only uses a wallet it made itself: a wallet that was already
#    in barkd's folder (made by hand, or by anyone else) is refused, because
#    someone else may hold its recovery words. Start barkd on an empty folder.
```

With the sidecar running, invoices get a real Ark address and a Lightning invoice
(best effort, via the ASP gateway), the watcher polls `/history` (barkd's current history
endpoint) with a persisted cursor, and the settlement engine sends the whole balance to
`EBP_SETTLEMENT_DESTINATION` (or the payout address saved in Settings) once it reaches the minimum.

---

## API

All `/api/v1/*` routes except `/health` require `Authorization: Bearer <api key>`.

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/v1/invoices` | Create an invoice (fiat-priced or sats-priced); optional `return_url` (http/https) adds a "Back to the shop" link on the checkout page |
| `GET` | `/api/v1/invoices/{id}` | Fetch one invoice (store-scoped) |
| `GET` | `/api/v1/invoices?status=&since=&limit=` | List invoices |
| `GET` | `/api/v1/invoices.csv` | Payments export (CSV; includes what the wallet actually received next to what was credited, and the invoice number and customer name as the last two columns, empty for shop orders) |
| `GET` | `/api/v1/admin/invoices` | Invoices tab (owner session): list, newest first; `q` searches number and customer name. Each invoice carries `payments` (oldest first: `rail`, `amount_sats`, `detected_at`, `counted`, `reason`, `handled`, and `unmatched_id`: the Alerts row of a payment that was not counted, so the invoice can mark it handled through `POST /api/v1/unmatched-payments/{id}/resolve`), joining `payments` and the `unmatched_payments` rows that name it, plus `late_sats` and `late_at` for money that was not counted. The CSV adds `late_sats` and `late_at` |
| `POST` | `/api/v1/admin/invoices` | Invoices tab: make an invoice (`sats` or `fiat`, `note`, `invoice_number`, `customer_name`; payable for 24 hours, `manual_invoice::OPEN_SECS`; the Lightning invoice asks for the time left and the Ark server may shorten it); refused while the tab is off |
| `GET` | `/api/v1/admin/invoices/settings` | Invoices tab: on or off, next number, customer names used before, store name, open hours, default currency, the currencies with a current price, the Ark server name (real-money mode only) and test mode; `prices` (price of one bitcoin in each of those currencies), `usd_per_btc` and `largest_sats` (largest invoice so far) let the page ask "is this right?" before an unusually large invoice (about $500 or more, or over 10 times the largest so far) |
| `POST` | `/api/v1/admin/invoices/enabled` | Turn the Invoices tab on or off (`{"enabled": true}`) |
| `GET` | `/api/v1/admin/invoices/{id}` | One invoice from the Invoices tab as it is now (404 for a shop order). The page asks every 15 seconds while an open invoice is on screen, so it turns Paid by itself; it stops when the invoice closes or the tab changes |
| `GET` | `/api/v1/admin/address` | Use your own address (in testing; `docs/OWN_DOMAIN.md`): the switch, main and backup addresses, the server's IP (from its sslip.io name), the suggested `pay.<shop domain>`, the record to add, the move's step, watch and backup days left, failures, and the host the shop last called with its key |
| `POST` | `/api/v1/admin/address/enabled` | The Settings switch (`{"enabled":bool}`) |
| `POST` | `/api/v1/admin/address/check` | `{"name"}`: does the name reach this server yet? Resolves it with the server's own resolver, spots Cloudflare's proxy, then fetches a one-time `/.well-known/ebp-check/<token>` page over plain http through the name. Changes nothing on the web server |
| `POST` | `/api/v1/admin/address/serve` | Checks again, then asks the address helper to serve the name next to the current address (Caddy gets its certificate) |
| `POST` | `/api/v1/admin/address/make-main` | `{"code"?}` (second step when two-step sign-in is on): checks the name over https with a valid certificate, then makes it the main address; the old one stays as the backup |
| `POST` | `/api/v1/admin/address/go-back` | The backup becomes the main address again |
| `POST` | `/api/v1/admin/address/keep-backup` | Postpones giving up the backup by 7 days |
| `POST` | `/api/v1/admin/address/cancel` | Before the switch only: stop the move and stop serving the new name |
| `GET` | `/.well-known/ebp-check/{token}` | Public: answers with the token while a check runs (random, 30 minutes). Caddy passes this path on over plain http for any host name; every other plain-http request is redirected to https |
| `POST` | `/api/v1/admin/invoices/{id}/lightning` | A fresh Lightning invoice, about 10 minutes after the old one ran out |
| `POST` | `/api/v1/admin/invoices/{id}/cancel` | Cancel an invoice made on the Invoices tab |
| `GET` | `/api/v1/admin/invoices.csv` | Invoices export: number and customer name first |
| `GET` | `/api/v1/admin/settlements.csv` | Transfers and upkeep export (CSV, oldest first; **admin session or admin token**): transfers to your own wallet and routine upkeep, with status, destination and note |
| `POST` | `/api/v1/invoices/{id}/cancel` | Cancel a pending invoice |
| `GET` | `/api/v1/unmatched-payments?limit=&offset=` | Payments that could not complete an order: part payments after expiry, extra payments on a paid order, or anything more than 3 hours late (needs merchant action). Each row has `source` (`shop`, `manual`, or `none` for money at an address of no payment request: reason `unknown_address`, or `exit_fees` (written already handled) for an address shown by `/admin/exit/deposit-address`, remembered in `own_addresses`). Every Lightning invoice ever issued is kept in `ln_invoice_history` (migration 0017), so a payment to a replaced one still finds its request. Rows for an invoice from the Invoices tab carry `invoice_number` and `customer_name`; a new row for a manual invoice queues the phone notice `invoice_paid_late` once (**admin session only**) |
| `POST` | `/api/v1/unmatched-payments/{id}/resolve` | Mark a late payment as handled (**admin session only**; the shop's API key is refused) |
| `GET` | `/api/v1/webhook-deliveries?status=&limit=&offset=` | Webhook delivery log (this store) (**admin session only**) |
| `POST` | `/api/v1/webhook-deliveries/{id}/resend` | Queue a delivery again (**admin session only**) |
| `GET` | `/api/v1/audit-log?limit=&offset=` | Security trail: key/webhook changes, resends, resolved late payments (**admin session only**) |
| `PUT` | `/api/v1/stores/webhook` | (**admin session only**) Set/clear the store's webhook URL; issues the signing secret once (`rotate_secret` to replace) |
| `GET` | `/api/v1/admin/version` | Current version, whether a newer release exists, open-order count |
| `POST` | `/api/v1/stores/webhook/test` | (**admin session only**) Send a harmless `webhook.test` notification to the store's webhook |
| `POST` | `/api/v1/stores/rotate-key` | (**admin session only**) Revoke all API keys for the store and issue a new one (shown once) |
| `GET` | `/api/v1/admin/status` | Operator view: counters, settlements, till (needs `EBP_ADMIN_TOKEN` bearer; 404 if unset) |
| `GET` | `/api/v1/admin/settlements` | Recent transfers to the owner's wallet (**admin session only**) |
| `GET` | `/api/v1/payout` | The saved payout address and any pending change (**admin session only**) |
| `GET` | `/api/v1/health` | Server, database, wallet, ASP reachability |
| `GET` | `/i/{id}` | Hosted checkout page |
| `GET` | `/i/{id}/json` | Full invoice JSON (fetched once by the checkout page) |
| `GET` | `/i/{id}/status` | Checkout status JSON (polled by the page) |
| `GET` | `/i/{id}/rails` | Which ways to pay this order offers (Lightning, Ark) |
| `GET` | `/admin`, `/dashboard` | Merchant admin page (password login; session cookie + CSRF) |
| `POST` | `/admin/login`, `/admin/login/passkey`, `/admin/logout`, `/admin/password` | Admin session endpoints; `GET /admin/session` reports login state |

### Admin page routes

All `POST`, all need a signed-in session and the CSRF header. Anything that moves money, changes who can sign in or changes where money goes also needs the admin password and a second step (an app code, a recovery code, or `pk:<ticket>` from a passkey approval). Every route that checks the password sits behind the password brake (`tests/password_brake_guard.rs` fails the build if one does not).

| Group | Routes |
| --- | --- |
| Payouts | `/admin/payout` (change address, waits a day), `/admin/payout/cancel`, `/admin/payout/min` (send minimum), `/admin/sweep` ("Move my money now") |
| Emergency withdrawal | `/admin/exit/status`, `/admin/exit/optin`, `/admin/exit/deposit-address`, `/admin/exit/prepare`, `/admin/exit/start`, `/admin/exit/cancel`, `/admin/exit/claim`, `/admin/exit/leftover` |
| Sign-in | `/admin/2fa/status`, `/admin/2fa/skip`, `/admin/2fa/start`, `/admin/2fa/confirm`, `/admin/2fa/disable`; `/admin/passkeys/status`, `/begin`, `/finish`, `/remove`, `/disable`, `/stepup/begin`, `/stepup/finish` |
| Shop | `/admin/pause`, `/admin/keylock/status`, `/admin/keylock/save` |
| Alerts and reconcile | `/admin/alerts/status`, `/enable`, `/disable`, `/test`, `/activity`; `/admin/reconcile/status`, `/admin/reconcile/done` |
| Backup and update | `/admin/backup`, `/admin/update/check`, `/admin/update/apply` |

### Invoice states

```
pending → paid | underpaid | overpaid | expired | cancelled
underpaid → paid | overpaid | expired
```

`overpaid` is treated as paid. An underpaid invoice keeps accepting payments
for 15 minutes after the first partial payment (and never past its own
expiry); if the total reaches the invoice amount it becomes `paid` (or
`overpaid`). A top-up after that window lands in Unmatched payments for you to
reconcile. Refunds are out of scope for v1.0 — Bitcoin payments are final, and the
checkout page says so.

### Webhooks

Every delivery carries:

```
X-EBP-Event: invoice.paid
X-EBP-Idempotency-Key: inv_...:invoice.paid:61000
X-EBP-Signature: t=1700000000,v1=<hex HMAC_SHA256(secret, "<t>.<body>")>
```

Stores must verify the HMAC with their per-store secret, reject timestamps older
than 5 minutes, dedupe on the idempotency key, and treat
`GET /api/v1/invoices/{id}` as the source of truth before fulfilling.

The webhook emitter performs a single delivery attempt. Retries with backoff,
the delivery log, and one-tap re-send are future work.

---

## Configuration

Resolution order: built-in defaults → optional TOML → `EBP_*` environment
variables. See [`ebp.example.toml`](../../ebp.example.toml) for every key.

| Variable | Default | Meaning |
| --- | --- | --- |
| `EBP_BIND_ADDR` | `0.0.0.0:8080` | Listen address |
| `EBP_DATABASE_PATH` | `./data/ebp.sqlite3` | SQLite file |
| `EBP_PUBLIC_BASE_URL` | `http://localhost:8080` | Used to build checkout links |
| `EBP_DEFAULT_EXPIRY_MINUTES` | `20` | Rate-lock lifetime |
| `EBP_DEFAULT_CURRENCY` | `EUR` | Fallback currency |
| `EBP_ONCHAIN_SWEEP_MAX_FEE_SAT_VB` | `50` | An automatic on-chain withdrawal waits until network fees are at or below this (sat/vB). The manual "move my money now" button ignores it |
| `EBP_WATCHER_INTERVAL_SECS` | `10` | Wallet poll interval |
| `EBP_LOG_LEVEL` | `info,tower_http=info` | `tracing` filter |
| `EBP_KEY_ENCRYPTION_KEY` | unset | 64 hex chars (32 bytes). Key that encrypts webhook secrets at rest (ChaCha20-Poly1305). Unset = a 0600 `ebp.key` file is created next to the database. Generate with `openssl rand -hex 32`; keep it OUTSIDE the data volume in production |
| `EBP_CHECKOUT_POLICY_URL` | unset | Refund policy link on checkout |
| `EBP_CHECKOUT_ACCENT_COLOR` | `#f7931a` | Checkout branding |
| `EBP_BARKD_URL` | `http://127.0.0.1:3000/api/v1` | barkd sidecar REST API |
| `EBP_BARKD_TOKEN` | unset | barkd bearer token (`barkd secret show`); env-only |
| `EBP_SETTLEMENT_DESTINATION` | unset | Merchant's Ark or on-chain sweep destination. Checked at startup like any saved address: a typo or an address for the wrong network (mainnet on a test server, or the reverse) stops the server from starting |
| `EBP_SETTLEMENT_INTERVAL_SECS` | `300` | Settlement engine cadence |
| `EBP_CONFIG` | unset | Path to an extra TOML config file (see `ebp.example.toml`) |
| `EBP_RENEWAL_LEAD_TIME_SECS` | `604800` | The earliest EBP refreshes a coin before it expires. EBP also reads the Ark server's coin lifetime and its free-refresh window (`GET /wallet/ark-info`) and waits until the coin is inside the free window, but never later than a few hours before expiry; if the server's terms cannot be read this value is used as is |
| `EBP_LIGHTNING_RECEIVE_ENABLED` | `true` | LN invoice per checkout invoice |
| `EBP_MODE` | `test` | The setup page's default network: `test` (signet, play money) or `live` (mainnet, real money, experimental). The owner chooses on the setup page; once the wallet exists that choice is saved (`server_mode` setting) and always wins over `EBP_MODE`, because a wallet belongs to one network for good. |
| `EBP_RATES_TIMEOUT_SECS` | `4` | How long each Bitcoin-price source gets to answer. The price is fetched in the background (see "The Bitcoin price" below), so this never slows a page down |
| `EBP_MAX_EXPIRY_MINUTES` | `1440` | Upper bound on invoice lifetime |
| `EBP_WEBHOOK_TIMEOUT_SECS` | `10` | Webhook delivery timeout |
| `EBP_WEBHOOK_ALLOW_PRIVATE` | unset | `1` lets webhook URLs target loopback/private addresses, and turns off the connection-time check that a website name points at a public address (local dev, or a shop on your own private network; env-only, not in the TOML) |
| `EBP_MAX_INVOICE_SATS` | `100000000` | Largest single payment request the server accepts (1 up to 21 million BTC in sats; the shop's key cannot raise it) |
| `EBP_INVOICE_RATE_LIMIT_PER_MIN` | `60` | New payment requests one shop key may make per minute; over it the server answers 429 `rate_limited` (`0` disables) |
| `EBP_PUBLIC_RATE_LIMIT_PER_MIN` | `120` | Per-client request limit on public `/i/*` checkout routes (`0` disables) |
| `EBP_TRUST_PROXY_HEADERS` | `false` | Use `X-Forwarded-For` as the rate-limit client key (only behind a proxy that overwrites it) |
| `EBP_UPDATE_CHECK` | `true` | Let the admin page check the public releases page for a newer version (one anonymous cached request). EBP never updates itself |
| `EBP_MANAGED_BARKD` | `false` (`true` in the default Docker image) | EBP runs the barkd wallet daemon itself, so setup needs no token copying or terminal commands. Use `docker-compose.managed.yml` |
| `EBP_ALLOW_INSECURE_SETUP` | `false` | The setup wizard refuses to run over plain `http://` on a non-local address (your recovery words and keys would cross the network unprotected). Set `true` only on a trusted LAN or for throwaway testing |
| `EBP_SETUP_TOKEN` | unset | Optional fixed setup code for automation. Normally leave it unset: until setup is finished the server makes a one-time **SETUP CODE** on every start, prints it where the server runs (`docker compose logs ebp`), and the setup page asks for it |
| `EBP_ADMIN_TOKEN` | unset | Optional static bearer token for the operator-only `GET /api/v1/admin/status` (404 when unset). It does not sign anyone in to the admin page; that is the password, two-step sign-in and passkeys |
| `EBP_PAYOUT_CHANGE_DELAY_SECS` | `86400` | How long a payout-address change waits before it applies (`0` = at once, for tests) |
| `EBP_ALERTS_SERVER` | `https://ntfy.sh` | The ntfy server phone alerts use unless the owner saved another one in Settings |
| `EBP_BARKD_BIN` | `barkd` | Path of the barkd program in managed mode |
| `EBP_UPDATES_DIR` | `/updates` | Where the update helper's request and answer folders are mounted |

**Retired settings.** `EBP_HOT_WALLET_CAP_SATS` and `EBP_ONCHAIN_SWEEP_CAP_SATS` (and `hot_wallet_cap_sats` / `onchain_sweep_cap_sats` in the config file) were removed on 19 November 2026. If still set they are ignored, and the server says so when it starts. The one rule for what stays on the server is the emergency-withdrawal opt-in below.

**What stays on the server, and when the rest is sent (from 19 November 2026).** By default the server keeps **nothing**: when the till holds at least the **send minimum**, the whole balance goes to the payout address in one send. That also leaves no small "change" coin behind (Bark 0.9 can refuse to spend coins under about 1,300 sats). The minimum has a hard floor per kind of payout address, `payout::Rail::floor`: **1,500 sats for Ark** (`ARK_MIN_FLOOR_SATS`) and **50,000 sats for on-chain** (`ONCHAIN_MIN_FLOOR_SATS`). The owner can raise either in **Settings → Where your money goes → When to send my money** (`POST /admin/payout/min` with `kind` `ark` or `onchain` and `min_sats`; null goes back to the floor): signed in, CSRF, admin password, second step when on, in the password-brake group; takes effect at once; security log `payout.min_changed`. Stored as `ark_sweep_min_sats` / `onchain_sweep_min_sats`; a stored value under its floor is ignored. Under the floor the route answers 400 with a plain explanation (`payout::floor_message`) before the slow password check, and the page shows the same words. An automatic on-chain send also waits for `EBP_ONCHAIN_SWEEP_MAX_FEE_SAT_VB`. "Move my money now" ignores the minimum and the fee limit.

Coins that the wallet has registered for a renewal round are never sent: barkd can list them as spendable for up to about an hour. The automatic transfer waits (and forgets its refusal) while any such coin is in a round, for at most `REQUESTED_GIVE_UP_SECS` (2 hours); "Move my money now" sends only the free coins and says so if nothing else is left.

**The emergency withdrawal is opt-in.** The setting `exit_opt_in` (`emergency_exit::opted_in`) is off by default; `POST /admin/exit/optin` (`on` true/false, password, code) turns it on (needs two-step sign-in, like the withdrawal itself) or off (password, plus a code when two-step is on; refused while a withdrawal runs). While it is on:

1. There is no way to keep Ark money back (removed 20 November 2026; the old `sweep_cap_sats` setting and `/admin/payout/cap` route are gone, and an old stored value is simply never read). Sweeps always send everything that is free, and an emergency withdrawal takes out everything on the server. By default little is there between sends, so there may be little to withdraw (the IF_SOMETHING_LOOKS_WRONG guide says so).
2. The server keeps at least `RESERVE_FLOOR_SATS` (5,000) of regular Bitcoin in barkd's own on-chain wallet for exit fees. No sweep touches it, and Home warns when the confirmed amount is under the floor.
3. `POST /admin/exit/leftover` (the **Send back what is above 5,000 sats** button) returns the extra: `emergency_exit::leftover_plan` takes the confirmed balance, less 5,000, less the fast network fee rate times `LEFTOVER_FEE_VBYTES` (600), padded by `LEFTOVER_FEE_PAD_PERCENT` (150%), and refuses with an explanation when under `MIN_LEFTOVER_SEND_SATS` (1,000) would be left to send. It sends through barkd `POST /onchain/send` (`{destination, amount_sat}`).

**Send journal (`wallet::settlement`).** Every Ark send or on-chain offboard first writes a `settlements` line with status `sending` (detail: the coin ids for an offboard), then updates it to `broadcast`. A clear refusal (error answer, connection never opened, update-needed text) deletes the line and goes through `record_failure` as before. A possibly-delivered request (`send_outcome_unknown`: timeout, "request failed", unreadable answer) leaves the line `unknown`; a `sending` line found at the start of a cycle (a restart mid-send; sends run one at a time under `CYCLE_LOCK`) counts the same. While one exists no transfer is sent, timer or button: `settle_from_history` marks it `broadcast` when barkd's history has exactly one outbound entry naming the destination, with an amount up to 10,000 sats above ours and a time not older than the attempt minus 5 minutes, and `failed` for an on-chain send whose named coins are all still free 10 minutes on; anything else waits for the owner (`POST /admin/transfer-outcome`, `arrived` or `not_arrived`, session only, audit `transfer.outcome`). One `transfer_unknown` phone notice per line (`transfer_unknown_alerted`). Home check id `transfer_unknown`.

**Payment-save trouble (`watcher`).** A non-permanent failure while recording a payment sets `payment_save_trouble_at` (cleared by the next clean tick), queues `payment_not_saved` once after 300 seconds, and shows the Home check `payment_not_saved`. A permanent rejection (`BadRequest`/`Conflict`) is parked in `unmatched_payments` with reason `could_not_be_recorded`.

While it is off, or once a withdrawal has been started, the same route sends everything through barkd `POST /onchain/drain` (the button reads **Send leftover to my address**). It is refused while a withdrawal is still running (the money may be needed for fees). Its destination is the address saved when the withdrawal started, never a later one.

Starting, stopping and claiming a withdrawal, and any exit already in progress, never depend on the switch, so an emergency is never blocked by it. The technical rules for starting one are unchanged: at least `MIN_EXIT_SATS` (50,000) on the server, a fresh fee estimate worth it (`MIN_FEE_MULTIPLE` 2), and confirmed fee money that covers the fees (`RESERVE_MIN_SATS` 1,500 is the technical minimum; 5,000 to 10,000 is recommended).

---

### The Bitcoin price

One price, one path (`src/rates.rs`). A background task asks five public sources
(mempool.space, blockchain.info, CoinGecko, Kraken, Coinbase) at once every 10
minutes, 4 seconds each, for USD, EUR, GBP, CHF, CAD, AUD and JPY, takes the median per
currency, and stores it in memory and in the `settings` table (`btc_price`) so a restart
keeps it. Everything else only reads the stored value, so no page or API call ever waits on
an outside website. The same price writes "1,000 sats ($0.40)" and converts orders priced in
a currency into sats (an order can be a little cheaper or more expensive than the very latest
price: accepted, in exchange for never failing because of a price site).

- Any sources failing is fine; none answering keeps the last good price.
- A new price more than 25% from one under an hour old waits for the next round.
- After 24 hours without a refresh the price is dropped: dollars disappear and currency-priced
  orders get a 503 `unavailable` (sats-priced orders are unaffected). Home warns after one hour.
- There is no built-in table of prices, in any mode. At 10-minute pace each source sees about
  4,300 requests a month; one request per source returns all currencies.

## Docker (VPS-first)

The supported deployment is a Docker Compose stack on a VPS with a public IP. The recommended, beginner-tested path is [DigitalOcean with a pasted setup block](../START_HERE.md); `./scripts/start.sh --https` adds the free padlock address automatically (bundled Caddy); home/Tor-only hosting is out of scope.

```bash
docker build -t easy-bitcoin-payments .
docker run --rm -p 8080:8080 -v ebp-data:/data easy-bitcoin-payments
docker run --rm -v ebp-data:/data easy-bitcoin-payments store create --name "My Shop"
```

The image runs as a non-root user, stores its database on the `/data` volume,
and exposes a healthcheck against `/api/v1/health`. `docker-compose.yml` also runs it
with a read-only filesystem (writable: `/data` and a 64 MB no-exec `/tmp`), all Linux
capabilities dropped and `no-new-privileges`; Caddy keeps only `NET_BIND_SERVICE`.
`scripts/verify_container_lockdown.sh` checks this on a running stack (CI runs it with
`--with-barkd`, which also starts barkd in a throwaway folder under the same limits).

---

## Development

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --all-features
```

Every module with logic has unit tests: the invoice state machine, API-key
hashing and authentication, webhook HMAC signing and verification, fiat→sats
conversion, database migrations, and the watcher's payment application path.

---

## Security notes

- **API keys are hashed at rest** (SHA-256) and shown exactly once at creation.
- **Secrets are never logged.** Webhook secrets and API keys do not appear in
  log lines or error responses.
- **Webhook secrets are encrypted at rest** (ChaCha20-Poly1305, random nonce,
  store id bound as associated data) with `EBP_KEY_ENCRYPTION_KEY`, or a
  `0600` key file `ebp.key` beside the database when unset. Set the env key
  from outside the data volume in production: the key file only protects
  against leaks of the SQLite file alone. Back the key up separately from the
  database (docs/RECIPE_BACKUP.md); if it is lost, re-issue secrets with
  `PUT /api/v1/stores/webhook {"rotate_secret": true}`. Rows from older
  versions are encrypted automatically at startup.
- **No customer PII is required.** An invoice needs an amount and an opaque
  order reference.
- **Custody (Level 0).** barkd holds the keys on the server; the server's
  bearer token can spend what the till holds but cannot touch anything already
  sent to your wallet. The whole balance goes out automatically once it reaches
  the minimum (and ahead of coin expiry), so a compromised server can steal at
  most what is in the till at that moment, plus the fee money if the emergency
  withdrawal is on. The payout address takes a day to change and a change needs
  your password and a second step, and a hacked shop cannot do either
  ([SHOP_COMPROMISE.md](SHOP_COMPROMISE.md)).
- **Sign-in and protected actions.** The admin password (16+ characters, a small
  block list refuses obvious ones), two-step sign-in with an app code and
  recovery codes, and passkeys (beta, `src/passkey.rs`). Every screen that checks
  the password sits behind one brake: 5 wrong tries a minute and 30 an hour per
  device (a try that succeeds is given back, `ratelimit::limit_failed_tries`, so
  doing several things in a row is never held up),
  20 a minute in total for everyone not signed in, and three wrong codes in a row
  pause code tries for longer each time. A code approves one protected action.
  `tests/password_brake_guard.rs` fails the build if a route that calls
  `verify_login_slow(` is outside the braked group in `src/routes/mod.rs`.
- **Passkeys.** WebAuthn with no attestation and ES256 (checked with `ring`).
  Challenges live only in memory, are single use, expire after two minutes and are
  tied to their purpose and to the signed-in session. A passkey approval for a
  protected action is a one-use ticket tied to the session it was made in
  (`passkeys::ticket_binding`), handed back as `pk:<ticket>` in the action's code
  field. Passkeys sit on top of the app code (it must be on first); the recovery
  codes cover both, and `ebp admin reset-2fa` removes passkeys too.
- **The container is locked down.** Read-only filesystem, no extra Linux powers,
  `no-new-privileges`; see "Docker" above and `scripts/verify_container_lockdown.sh`.
- **No inline code in the pages.** The checkout, admin and setup pages load their
  script and stylesheet as files from the server (`assets/<page>.js` and
  `assets/<page>.css`, compiled in and served by `src/routes/page_assets.rs`), so
  the Content-Security-Policy is `script-src 'self'; style-src 'self'` with no
  `'unsafe-inline'`: a script injected into a page would not run. Keep it that way:
  no `<script>` without `src`, no `<style>` blocks, no `style=""` attributes and no
  `onclick=""`-style handlers. Setting styles from a script (`el.style.x = ...`) is
  fine. A Rust test checks the pages and the header, and the browser tests serve
  the real policy and fail on anything it blocks.
- **No uploads, ever, while the server runs.** The only pictures EBP shows are the
  success mascots in `assets/mascot/`, compiled into the program and served from a
  fixed list of thirteen names (`GET /assets/mascot/<name>.png`). No route accepts a file,
  an unknown name is a plain 404, and every other method gets 405. The checkout page
  picks one of ten mascots from the payment id and only ever builds image addresses
  from that fixed set. A test (`mascot_art_is_built_in_read_only_and_cannot_be_uploaded`)
  guards this. The Home tab reuses one of these pictures (the weightlifter) for a short cheer
  when a payment arrives: it reads the payments list the Payments tab already reads, at most
  once a minute, and ignores every failure (`tests/e2e/payment_cheer.py`). To change the art, change the files in the repository and release a new build.

---

## License

MIT. See [`LICENSE`](../../LICENSE).

## EBP vs. BTCPay Server: an honest comparison

EBP and BTCPay Server share a mission: self-hosted, self-custodial, free Bitcoin
payments with no middleman between you and your customers. They differ in the
generation of technology underneath. BTCPay is a first-generation implementation:
Lightning channels, a mandatory full node, and a C#/.NET codebase with roughly
seven years of accretion. EBP is a second-generation attempt: Ark instead of
channels, a single Rust binary instead of a monolith, and a design target of a
merchant who has never opened a terminal. Both positions carry real tradeoffs.
This section states them bluntly, including where EBP loses.

| | BTCPay Server | EBP |
| --- | --- | --- |
| Stack | C#/.NET monolith, ~7 years of accretion | Rust, single binary + SQLite |
| Payment rails | Real Lightning channels + on-chain | Ark via a barkd sidecar; Lightning via the ASP gateway; on-chain fallback |
| Inbound liquidity | Required — the #1 Lightning merchant pain | None — Ark receive is passive |
| Bitcoin node | Full node required | None |
| Ops weight | Heavy: chain sync, channel management, multi-service updates | One binary + one sidecar; no chain sync |
| Wallet model | Server-held Lightning keys | Capped till + automatic sweeps — hot by design, and the UI says so |
| Setup | Sysadmin-grade | One-click deploys + AI-assisted recipe |
| Lightning trust | Zero third party on the payment path | ASP dependency + gateway trust window until round refresh |
| Feature breadth | POS, crowdfunding, pay buttons, plugins | Invoice API, checkout, store connectors |
| Maturity | Thousands of merchants in production | Pre-release |
| Management | Desktop-first | iPhone-first |

### Where BTCPay wins — stated honestly

- **Real Lightning channels.** No third party sits on the payment path. EBP's
  Lightning receive runs through the ASP's gateway and carries a trust window
  until the received VTXO is refreshed in a round, and Ark itself depends on
  the ASP for liveness and coordination. On trust-minimization, BTCPay wins.
- **Battle-tested maturity.** Thousands of merchants, years in production,
  edge cases found and fixed. EBP is pre-release — barkd's response shapes
  have fixture tests, but no independent audit yet.
- **Ecosystem breadth.** In-person POS app, crowdfunding, pay buttons, a
  plugin system. EBP ships an invoice API, a checkout page, and store
  connectors; POS-lite is v1.x.

If you are a trust-minimization purist, use BTCPay.

### Where EBP wins

- **No node, no channels, no inbound liquidity.** Nothing to sync, nothing to
  open, nothing to rebalance. The #1 operational barrier to accepting
  Lightning payments does not exist here.
- **Setup for people who have never opened a terminal.** One-click deploys
  are a spec requirement, not an afterthought, with an AI-assisted recipe
  on top.
- **iPhone-first management.** Payments, settlement, the expiry queue, and
  health alerts are designed to be handled from a phone. If a flow cannot
  be completed on a phone, it does not ship.
- **Smaller breach blast radius.** The server holds a small till that is sent whole to your own wallet once it
  reaches a minimum (1,500 sats with an Ark payout address, 50,000 with an on-chain one), automatically — the UI
  calls it "cash in a till; we empty it into your vault automatically." A
  compromised server can steal the float, not a full Lightning node's hot
  wallet.
- **A modern, maintainable codebase.** One Rust binary and a SQLite file are
  easier to audit, reason about, and patch than a large C# solution stack.

### Who should pick which

Pick **BTCPay** if you want maximum trust-minimization on Lightning, need
in-person POS or crowdfunding today, or already run a node and are comfortable
as a sysadmin. If you enjoy running a full node, BTCPay is the better tool
for you.

Pick **EBP** if you run (or want to run) an online store, have no node and do
not want one, do not want to manage channels, value running the whole
operation from an iPhone, and accept the ASP and gateway trust windows
documented in the spec.

EBP trades protocol purity for accessibility on purpose — [spec.md](spec.md)
documents every tradeoff in full.

### Incidents inform design

In August 2026, a BTCPay Server vulnerability let unauthenticated attackers
read LND admin macaroon files, and they drained merchants' Lightning channels
(foundationdevices.com and others confirmed losses). The failure class:
a high-value signing credential reachable through the web tier, guarding a
hot wallet whose size is a *requirement* of Lightning (channels must hold
liquidity). EBP is designed against that failure class:

- The server-side wallet is a **small till**, sent whole to your own wallet once it reaches a minimum (1,500 sats
  with an Ark payout address, 50,000 with an on-chain one). A compromised server exposes the
  float, not the vault.
- **You do not have to trust the ASP with your funds — only for liveness.**
  Ark is self-custody: you hold the keys to your VTXOs. The Ark Service
  Provider coordinates payment rounds and keeps the service responsive; it
  cannot spend your money. If the ASP disappears or misbehaves, you take
  your funds home with a unilateral exit — slower, but yours. The worst an
  ASP can do to you is *stall* you, not steal from you.
- The barkd bearer token is the one server credential, bound to loopback,
  never logged, and its blast radius is the till cap — not a full Lightning
  node's hot wallet and channels.

The honest counterweight: EBP is unaudited, pre-release, and adds the ASP
liveness dependency. To be precise about the security claim: **we do not
think EBP is more secure than BTCPay Server.** We think it is **lower risk,
on purpose** — an intentional tradeoff that puts a smaller amount of money
in harm's way and constrains what the exposed components can do. A smaller
attack surface with a smaller prize is not the same thing as a stronger
vault; it is a different bet, made deliberately and documented in
`spec.md`.

