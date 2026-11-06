# Changelog

## Unreleased (open review items — see docs/REVIEW_2026-11-05.md)

### Security
- Cleanup: all GitHub Actions pinned by commit SHA plus a weekly Dependabot PR (never auto-merged); release `SHA256SUMS` signed with sigstore; over https the admin session cookie is `__Host-ebp_session` (a plain cookie of that name is ignored).
- **Webhook secrets are now encrypted at rest** (ChaCha20-Poly1305; `EBP_KEY_ENCRYPTION_KEY` or a 0600 `ebp.key`). New dependency: `chacha20poly1305`. Back the key up separately from the database.
- Audit trail (`audit_log`, `GET /api/v1/audit-log`) for key rotation, webhook changes, re-sends and resolved late payments; webhook delivery log + re-send endpoints.

### Release
- Removed the two-container install (`docker-compose.two-container.yml`, its walkthrough, `scripts/backup.sh`, the old two-container setup recipe). One install path; the admin page's backup download replaces the script. Test count in the README refreshed.
- The release workflow can be dry-run from the Actions tab (builds and tests; publishes, pushes and signs nothing unless a `v*` tag).
- `docker-compose.yml` accepts `EBP_IMAGE` (a signed prebuilt image); `scripts/start.sh` pulls it and falls back to building.

### Fixed
- Wizard wallet-create now sends the field barkd actually reads (`ark_server`); a custom ASP was silently ignored before.
- Barkd request shapes aligned with the pinned OpenAPI (`amount_sat`, `vtxos`, `tip_height`); wallet history read from `/api/v1/history` (the old endpoint is deprecated).
- Payments completed after an invoice's deadline are parked as `paid_after_expiry` regardless of watcher tick order.

### Added
- Payout address (where the till empties to): asked for in the setup wizard and editable in Settings. The first address applies at once; changing it later needs the admin password, waits 24 hours (`EBP_PAYOUT_CHANGE_DELAY_SECS`), is logged, shows a warning on Home and can be cancelled. On-chain addresses get a checksum check. An address in the server configuration still wins.
- One install path: `docker-compose.yml` is now the single-container install (EBP runs the wallet daemon itself); the old two-container stack is `docker-compose.two-container.yml` with its own guide (`docs/ADVANCED_TWO_CONTAINER.md`). The AWS walkthrough is rewritten for the single path (setup code, no token copying, no `.env`); no `COMPOSE_FILE` step is needed any more.
- The encrypted backup now includes the at-rest encryption key (`ebp.key`), so one file restores everything; START_HERE no longer asks beginners to generate and save a key.
- Signet drill: pre-flight checklist, new steps (https cookie, Try a payment, backup download/verify/restore against the real barkd folder) and `scripts/drill_capture.sh`, a read-only helper that saves barkd's real answers (never tokens or recovery words) for comparison with the parser.
- Beginner docs round 2 (second independent review): START_HERE now has a "choose where to run it" box (this computer vs a server + tunnel), Docker/git/terminal prerequisites, wallet and faucet, a `.env` that pins the single-container compose file and creates the encryption key, a test-payment and a backup section; MASTER walkthrough uses the Try a payment button instead of `curl`; managed compose passes the sweep destination and cap; `start.sh` no longer claims setup is finished when the first build is just slow; setup wizard says to leave the network fields alone.
- Beginner docs pass (from an independent role-play review): new `docs/START_HERE.md` (honest status, what you need, single-container install, "how you know it worked", glossary); walkthrough now covers the setup code (`start.sh`), the admin password and the webhook secret, and no longer overwrites `.env`; dead references in the backup recipe marked as not-yet-available.
- Admin Payments tab: "Try a payment" creates a test payment request (amount in sats) and links to its checkout page, so a shop owner can see a payment end to end without any command line. Covered by the browser e2e.
- Encrypted backup download: Settings tab → "Download a backup" (locked with the admin password, so there is nothing new to remember; not stored). One file holds a consistent database snapshot (signed-in sessions removed) and, in managed-barkd mode, the wallet folder; ChaCha20-Poly1305 with a PBKDF2 (600k) key. `ebp backup verify <file>` and `ebp backup restore <file> <new-folder>` (never overwrites; integrity-checks the database). Covered by unit tests, an API test and the browser e2e.
- Checkout shows a QR code (vendored same-origin generator, `/assets/qr.js`).
- `PUT /api/v1/stores/webhook` and `POST /api/v1/stores/rotate-key`.
- On-chain confirmation gate: payments are matched by output script from `/onchain/transactions` and credited only at `EBP_ONCHAIN_MIN_CONFIRMATIONS` (default 1) depth; an invoice with a payment seen before its deadline is held open (max 24h) and is not treated as late. Checkout shows "waiting for confirmation". History-feed on-chain receipts are never credited directly.
- Admin Home tab now opens with an "Is everything OK?" checklist (wallet connected, HTTPS, safety reserve, coins near expiry, late payments, shop notifications, last withdrawal, large balance) in plain words, worst first; jargon ("sweep", "delivery log") replaced. Rules live in `src/health_checks.rs` with unit tests; `GET /api/v1/admin/status` returns them as `checks`.
- Browser end-to-end test in CI (docs/E2E_TESTING.md): real binary + spec-validating fake barkd + Chromium through the wizard, admin page, Ark/on-chain/RBF/underpay payments, outage and restart. Found and fixed: plain Ark-address payments had no dedupe key in the spec'd history shape and would not have been credited (now keyed on the received coin).
- Security review fixes (docs/SECURITY_REVIEW_2026-11-05.md): recovery-words window closed on every error path; login hashing off the database lock; proxy-safe rate limiting plus a global login cap; stronger webhook address filter; compose publishes on loopback by default (`EBP_BIND_HOST`); owner-only database/backup permissions; least-privilege workflow tokens.
- Bark audit: field-level contract check against the pinned OpenAPI (`docs/barkd-contract.json`, `scripts/check_barkd_contract.py`, run in CI); removed the mistyped, unused `chain_source` request field; findings in `docs/BARK_AUDIT.md`.
- Updates, merchant-controlled only: Home tab shows the version and an "Update available" / "Security update" notice with host-specific steps and the open-order count (`GET /api/v1/admin/version`; one anonymous cached lookup, `EBP_UPDATE_CHECK=false` to disable). EBP never updates itself; `render.yaml` sets `autoDeploy: false`. The database is copied to `backups/` (last 3) before any upgrade, and an older version refuses to run on newer data. Release workflow now signs images (cosign keyless), attaches provenance + SBOM, drops the `latest` tag and puts the digest and verify command in the release notes.
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
