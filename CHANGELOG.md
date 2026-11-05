# Changelog

## Unreleased (open review items — see docs/REVIEW_2026-11-05.md)

### Fixed
- Wizard wallet-create now sends the field barkd actually reads (`ark_server`); a custom ASP was silently ignored before.
- Barkd request shapes aligned with the pinned OpenAPI (`amount_sat`, `vtxos`, `tip_height`); wallet history read from `/api/v1/history` (the old endpoint is deprecated).
- Payments completed after an invoice's deadline are parked as `paid_after_expiry` regardless of watcher tick order.

### Added
- Checkout shows a QR code (vendored same-origin generator, `/assets/qr.js`).
- `PUT /api/v1/stores/webhook` and `POST /api/v1/stores/rotate-key`.
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
