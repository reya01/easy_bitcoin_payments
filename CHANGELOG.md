# Changelog

## Unreleased (open review items — see docs/REVIEW_2026-11-05.md)

### Security
- **Webhook secrets are now encrypted at rest** (ChaCha20-Poly1305; `EBP_KEY_ENCRYPTION_KEY` or a 0600 `ebp.key`). New dependency: `chacha20poly1305`. Back the key up separately from the database.
- Audit trail (`audit_log`, `GET /api/v1/audit-log`) for key rotation, webhook changes, re-sends and resolved late payments; webhook delivery log + re-send endpoints.

### Fixed
- Wizard wallet-create now sends the field barkd actually reads (`ark_server`); a custom ASP was silently ignored before.
- Barkd request shapes aligned with the pinned OpenAPI (`amount_sat`, `vtxos`, `tip_height`); wallet history read from `/api/v1/history` (the old endpoint is deprecated).
- Payments completed after an invoice's deadline are parked as `paid_after_expiry` regardless of watcher tick order.

### Added
- Checkout shows a QR code (vendored same-origin generator, `/assets/qr.js`).
- `PUT /api/v1/stores/webhook` and `POST /api/v1/stores/rotate-key`.
- On-chain confirmation gate: payments are matched by output script from `/onchain/transactions` and credited only at `EBP_ONCHAIN_MIN_CONFIRMATIONS` (default 1) depth; an invoice with a payment seen before its deadline is held open (max 24h) and is not treated as late. Checkout shows "waiting for confirmation". History-feed on-chain receipts are never credited directly.
- Admin page Connect tab: guided four-step "connect your shop" (server address, API key, shop webhook address + signing secret, test notification). `POST /api/v1/stores/webhook/test` sends a harmless `webhook.test` event so the merchant can confirm the shop receives and verifies it.
- Setup wizard backup check, hardened: safety checklist before the words show; words visible 60 s at a time and wiped on hide (the page keeps no copy); three random words typed into password-style fields; checked SERVER-side (`POST /setup/wallet/verify-backup` re-reads the words from barkd for that request) and required before `confirm-backup`.
- Rolling cleanup (`retention`): every 6 hours the server trims operational logs — failed-login rows (14 days, max 500), audit trail (365 days, max 20,000 rows), delivered webhook records (90 days), failed ones (180 days), on-chain sightings of finished invoices (90 days), expired sessions. Invoices, payments, unmatched payments and settlements are never deleted.
- Admin page (`/admin`): mobile-first sign-in with a password chosen in the setup wizard (PBKDF2-HMAC-SHA256, 600k rounds). HttpOnly + SameSite=Strict session cookie, CSRF header on every write, login limited to 10/min per client, sessions end on password change. Shows health, recent payments, payments needing attention, webhook settings and delivery log, API-key rotation, password change and the security trail. `ebp admin set-password` resets the password on the server (the only recovery; there is no remote reset by design).
- Single-container install (`EBP_MANAGED_BARKD`, default Dockerfile target, `docker-compose.managed.yml`, `render.yaml`): EBP runs barkd itself, reads its token, performs the recovery-words ceremony and restarts once after setup, so the wizard needs no terminal steps. Setup refuses plain `http://` off-localhost (`EBP_ALLOW_INSECURE_SETUP`). See docs/BEGINNER_ROADMAP.md.
- CLI recovery: `ebp store list` and `ebp store rotate-key [--store ID]`; `scripts/start.sh` prints the one-time setup code in its final message.
- `limit`/`offset` on `GET /api/v1/unmatched-payments`; `order_id` (128 B) and `metadata` (4 KB) limits on invoice create.
- Settlement: on-chain sweeps offboard only the VTXOs needed to cover the excess; a short exit reserve triggers a throttled consolidating refresh.

## Beta hardening (PR #1 — see docs/BETA_PLAN.md)

### Fixed
- Settlement engine compared barkd block-height expiries as timestamps, refreshing every VTXO every tick. Heights now resolve through the chain tip; unknown expiry is throttled to hourly.
- Repeated refused sweeps no longer write a `settlements` row every tick.
- Payments arriving after an invoice expired/closed were silently dropped; they are now recorded in `unmatched_payments` (migration 0005).

### Added
- `GET /api/v1/admin/status` operator endpoint (bearer `EBP_ADMIN_TOKEN`).
- Optional `EBP_SETUP_TOKEN` guarding first-run wizard actions.
- Durable webhook outbox: events are queued in `webhook_deliveries` (migration 0006) and delivered with exponential backoff, surviving restarts.
- Rate limiting for public checkout routes (`public_rate_limit_per_min`, `trust_proxy_headers`).
- `GET /api/v1/unmatched-payments` and `POST /api/v1/unmatched-payments/{id}/resolve` to review and close out late payments.

### Changed
- **On-chain rail is off by default** (`onchain_rail_enabled`, env `EBP_ONCHAIN_RAIL_ENABLED=true` to enable): no tab, no address in the QR/API, and any on-chain payment that arrives is parked in `unmatched_payments`.
- Invoice creation runs its synchronous wallet calls on the blocking pool; fiat amounts above the money supply are rejected up front.
- `POST /api/v1/invoices` is idempotent per `order_id`: a retry with the same store, order and amount returns the existing open invoice with HTTP 200 instead of creating a duplicate (201).
- `GET /api/v1/health` adds `watcher_last_tick_age_secs`.
- Watcher and settlement ticks are supervised: a panic is logged and the loop continues. Barkd bridge failures return `WalletError::Unavailable` instead of panicking.

### Security
- Webhook URLs are validated (no private/loopback/link-local targets, http/https only) when set via the CLI; the emitter never follows redirects.
- Webhook delivery is decoupled from the watcher (durable outbox) so a slow or dead merchant endpoint cannot stall payment detection.
- Synchronous wallet calls run on the blocking pool.
