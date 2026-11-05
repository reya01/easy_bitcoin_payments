# Changelog

## Unreleased (beta-hardening, PR #1 — see docs/BETA_PLAN.md)

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
- `POST /api/v1/invoices` is idempotent per `order_id`: a retry with the same store, order and amount returns the existing open invoice with HTTP 200 instead of creating a duplicate (201).
- `GET /api/v1/health` adds `watcher_last_tick_age_secs`.
- Watcher and settlement ticks are supervised: a panic is logged and the loop continues. Barkd bridge failures return `WalletError::Unavailable` instead of panicking.

### Security
- Webhook URLs are validated (no private/loopback/link-local targets, http/https only) when set via the CLI; the emitter never follows redirects.
- Webhook delivery is decoupled from the watcher (durable outbox) so a slow or dead merchant endpoint cannot stall payment detection.
- Synchronous wallet calls run on the blocking pool.
