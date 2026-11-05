# Beta hardening pass — status ledger

Branch: `beta-hardening` (draft PR #1). Written by Claude in a sandbox with
**no crates.io access**, so it cannot compile locally (`cargo fmt` does work
offline). **CI is the compiler**: push the branch and watch the `build + test +
clippy` job (fmt -> clippy -D warnings -> test -> release build). As of the
last push that job is GREEN for every row marked "CI-green" below. CI only
triggers on pull requests and pushes to `main`, so keep PR #1 open.

Workflow tips for other agents: run `cargo fmt` before every push (CI fails
on formatting first and skips the rest); fetch CI results with
`gh api repos/reya01/easy_bitcoin_payments/actions/runs/<id>/jobs` (GraphQL
and `gh pr create` are unavailable in Claude Code sessions; use REST).

## Done (CI-green: fmt, clippy -D warnings, tests, release build)

| # | Change | Files |
|---|--------|-------|
| 1 | **Settlement renewal bug.** barkd reports expiry as a block *height*; the old filter treated "no `expires_at`" as always due, so every VTXO was refreshed every tick. Heights now convert via chain tip (`vtxo_expiry_unix`, shared with `vtxo_queue`). Unknown expiry is refreshed at most hourly. Persistent refused sweeps write one `settlements` row, not one per tick. New tests: `height_expiry_*`, `expiry_conversion_handles_heights_and_unknowns`. | `src/wallet/barkd.rs`, `src/wallet/settlement.rs` |
| 2 | **Late payments no longer vanish.** Payments to the address of an expired/closed invoice are recorded in new table `unmatched_payments` (migration 0005) and logged at WARN. Merchant must reconcile (refund/re-credit) and set `resolved_at`. No UI/API yet. | `migrations/0005_*.sql`, `src/invoice.rs::record_unmatched`, `src/watcher.rs` |
| 3 | **Webhook SSRF guard.** `webhook::validate_webhook_url` rejects non-http(s), embedded creds, loopback/private/link-local IP literals, `localhost`. Applied in the `ebp store create --webhook-url` CLI; bypass with `EBP_WEBHOOK_ALLOW_PRIVATE=1`. Emitter no longer follows redirects. DNS-rebinding NOT covered. | `src/webhook.rs`, `src/main.rs` |
| 4 | **Durable webhook outbox** (supersedes the earlier in-memory retry task). The watcher only enqueues rows in `webhook_deliveries` (migration 0006, UNIQUE idempotency key); ONE dispatcher task (`webhook_outbox::run`, spawned in `main.rs`) delivers every 5 s with backoff 30s/5m/30m/2h/12h, 6 attempts total, then `failed`. Target URL re-validated at delivery (rejected = permanent failure). Survives restarts. No UI / manual re-send yet; inspect with `SELECT * FROM webhook_deliveries WHERE status != 'delivered';`. | `src/webhook_outbox.rs`, `migrations/0006_*.sql`, `src/watcher.rs`, `src/main.rs` |
| 5 | **Blocking wallet calls on the blocking pool** (`spawn_blocking`) in the watcher and `/api/v1/health`. | `src/watcher.rs`, `src/routes/health.rs` |
| 6 | **Invoice-create idempotency.** `POST /api/v1/invoices` with an `order_id` that already has an OPEN (pending, unexpired) invoice for the same store and same amount (fiat amount+currency for fiat requests, sats otherwise) returns that invoice with **200 OK** instead of creating a new one (201). Changed amount => new invoice. | `src/invoice.rs::find_open_by_order_id`, `src/routes/invoices.rs::create` |
| 10 | **Unmatched-payments API.** `GET /api/v1/unmatched-payments` and `POST /api/v1/unmatched-payments/{id}/resolve` (store API key; rows scoped via invoice->store). No dashboard UI yet. | `src/routes/unmatched.rs`, `src/routes/mod.rs` |
| 11 | **Public-route rate limiting.** In-memory fixed-window limiter (`public_rate_limit_per_min`, default 120/client/min, 0=off) on `/i/*` only; 429 + `Retry-After`. Client key = peer IP (main.rs uses `into_make_service_with_connect_info`), or first `X-Forwarded-For` when `trust_proxy_headers=true`. Not a DDoS defence; resets on restart. API routes are NOT limited (they are API-key authenticated). | `src/ratelimit.rs`, `src/routes/mod.rs`, `src/config.rs`, `ebp.example.toml` |
| 12 | **Optional wizard setup token.** `EBP_SETUP_TOKEN` / `setup_token`: when set, every non-GET `/setup/*` request needs `X-EBP-Setup-Token` (constant-time compare); page + `/setup/status` stay open; `assets/setup.html` prompts on a 401 `setup_token_required` and retries. Compose passes `EBP_SETUP_TOKEN` through. Default unset = old behaviour (wizard open until setup completes). NOT yet browser-tested: the prompt flow in `setup.html` is covered only by the server-side test in `tests/api_flows.rs`. | `src/routes/setup.rs::setup_token_guard`, `src/routes/mod.rs`, `assets/setup.html`, `src/config.rs`, `docker-compose.yml` |
| 13 | **Operator status endpoint.** `GET /api/v1/admin/status` with `Authorization: Bearer $EBP_ADMIN_TOKEN` (a store API key is NOT accepted; 404 when no admin token is configured). Returns DB counters (pending invoices, open unmatched payments, pending/failed webhooks), last 10 settlements, and barkd figures: off/on-chain balances, exit-reserve check (confirmed on-chain vs 15,000 sat per spendable VTXO), optional `?funding_address=1` to get a fresh on-chain top-up address (consumes a wallet address per call). Wallet failures are reported in `wallet.error`; counters are still returned. | `src/routes/admin.rs`, `src/routes/mod.rs` |
| 14 | **Smaller fixes.** `POST /api/v1/invoices` wallet calls (`receive_addresses`, `lightning_invoice`) now run via `spawn_blocking`; `rates::fiat_to_sats` rejects conversions above `invoice::MAX_INVOICE_SATS` (21M BTC) early with a 400, matching the `sats` path. | `src/routes/invoices.rs`, `src/rates.rs` |
| 15 | **On-chain rail OFF by default, one switch to turn on.** `onchain_rail_enabled` / `EBP_ONCHAIN_RAIL_ENABLED=true` (compose passes it through). Off: checkout hides the On-chain tab, the BIP321 URI has an empty path (`bitcoin:?amount=..&lightning=..&ark=..`), API/checkout JSON return `onchain_address: null` (+ `onchain_enabled` flag on `/i/{id}/json`), and the watcher parks any on-chain-rail payment in `unmatched_payments` with reason `onchain_rail_disabled` instead of crediting. Wallet still derives an on-chain address per invoice internally. | `src/config.rs`, `src/routes/checkout.rs`, `src/routes/invoices.rs`, `src/watcher.rs`, `assets/checkout.html` |
| 7 | **Supervised background loops.** Watcher and settlement run each tick in its own spawned task; a panic is logged (`ERROR ... PANICKED; continuing`) and the loop continues. | `src/watcher.rs::run`, `src/wallet/settlement.rs::run` |
| 8 | **Bridge errors, not panics.** `BarkdWallet::block_on` returns `WalletError::Unavailable` for a dead/poisoned bridge and catches panics inside bridged futures. | `src/wallet/barkd.rs` |
| 9 | **Watcher heartbeat.** `/api/v1/health` now includes `watcher_last_tick_age_secs` (null until first successful tick). Alert if it grows well beyond `watcher_interval_secs`. | `src/watcher.rs::LAST_TICK_UNIX`, `src/routes/health.rs` |

## CI status note

Build + test + clippy: green on the latest push (all rows above). The
`docker image builds (amd64 + arm64)` job was still `in_progress` for a very
long time on the last runs (arm64 under emulation) and has NOT been seen to
complete from this sandbox; re-check it on the PR before relying on it. The
Dockerfile already copies `migrations/` and `assets/`, so new migrations and
the `setup.html` change are included.

## Not done — pick up here (priority order)

1. Webhook delivery log API / dashboard + manual re-send (the outbox table already has the data).
2. Dashboard UI for `unmatched_payments` (API + integration tests exist in `tests/api_flows.rs`).
3. Run the live signet on-chain receive drill (pay an invoice's on-chain address with the rail enabled, capture the real `/wallet/history` entry, pin its shape in `barkd.rs`, add a fixture test), THEN consider flipping the default to on. The rail is gated off by default (row 15).
4. Make the setup token mandatory in `mode = "live"` (decision for the maintainer), and browser-test the `setup.html` prompt.
5. Surface `/api/v1/admin/status` in the dashboard UI (currently API only) and add webhook re-send.
6. ~~Webhook secrets are stored in plaintext~~ — DONE: encrypted at rest (src/secrets.rs, ChaCha20-Poly1305).
7. Compose publishes `8080:8080` on all interfaces; no automated TLS.
8. Human gates: UX review, release tag, CHANGELOG sign-off, WooCommerce plugin scope.
