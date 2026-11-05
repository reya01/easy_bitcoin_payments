# Changelog

## Unreleased (beta-hardening — UNCOMPILED, see docs/BETA_PLAN.md)

### Fixed
- Settlement engine compared barkd block-height expiries as timestamps, refreshing every VTXO every tick. Heights now resolve through the chain tip; unknown expiry is throttled to hourly.
- Repeated refused sweeps no longer write a `settlements` row every tick.
- Payments arriving after an invoice expired/closed were silently dropped; they are now recorded in `unmatched_payments` (migration 0005).

### Security
- Webhook URLs are validated (no private/loopback/link-local targets, http/https only) when set via the CLI; the emitter never follows redirects.
- Webhook delivery runs in the background with retries instead of blocking the watcher.
- Synchronous wallet calls run on the blocking pool.
