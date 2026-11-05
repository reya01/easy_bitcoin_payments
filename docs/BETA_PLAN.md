# Beta hardening pass — status ledger

Branch: `beta-hardening` (3 commits on top of 77d4f95). Written by Claude in a
sandbox with **no crates.io access**, so NOTHING here has been compiled, linted
(`cargo fmt`/`clippy -D warnings`) or tested. **First action for any agent or
human picking this up: run `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
and fix whatever the compiler finds.** Treat every item below as "implemented,
unverified" until that passes.

## Done (unverified)

| # | Change | Files |
|---|--------|-------|
| 1 | **Settlement renewal bug.** barkd reports expiry as a block *height*; the old filter treated "no `expires_at`" as always due, so every VTXO was refreshed every tick. Heights now convert via chain tip (`vtxo_expiry_unix`, shared with `vtxo_queue`). Unknown expiry is refreshed at most hourly. Persistent refused sweeps write one `settlements` row, not one per tick. New tests: `height_expiry_*`, `expiry_conversion_handles_heights_and_unknowns`. | `src/wallet/barkd.rs`, `src/wallet/settlement.rs` |
| 2 | **Late payments no longer vanish.** Payments to the address of an expired/closed invoice are recorded in new table `unmatched_payments` (migration 0005) and logged at WARN. Merchant must reconcile (refund/re-credit) and set `resolved_at`. No UI/API yet. | `migrations/0005_*.sql`, `src/invoice.rs::record_unmatched`, `src/watcher.rs` |
| 3 | **Webhook SSRF guard.** `webhook::validate_webhook_url` rejects non-http(s), embedded creds, loopback/private/link-local IP literals, `localhost`. Applied in the `ebp store create --webhook-url` CLI; bypass with `EBP_WEBHOOK_ALLOW_PRIVATE=1`. Emitter no longer follows redirects. DNS-rebinding NOT covered. | `src/webhook.rs`, `src/main.rs` |
| 4 | **Webhook delivery off the watcher loop** with retries at 0s/30s/5m/30m in a detached task. In-memory only: a restart drops pending retries. | `src/watcher.rs` |
| 5 | **Blocking wallet calls on the blocking pool** (`spawn_blocking`) in the watcher and `/api/v1/health`. | `src/watcher.rs`, `src/routes/health.rs` |

## Not done — pick up here (priority order)

1. **Compile + CI green** (see top).
2. Durable webhook outbox (`webhook_deliveries` table; replace the in-memory retry task).
3. Invoice-create idempotency on `order_id` (duplicate WooCommerce retries create duplicate invoices).
4. Supervise `watcher.run()` / `engine.run()` in `main.rs` (a panic currently ends the task silently); surface a heartbeat in `/health`.
5. Convert `BarkdWallet::block_on` panics into `WalletError`.
6. Gate the on-chain rail until the on-chain `/wallet/history` receive shape is pinned by a live signet drill (still UNPINNED).
7. Rate limiting on public checkout routes; optional setup token for the unauthenticated wizard.
8. Admin status endpoint (`EBP_ADMIN_TOKEN` is parsed but unused) incl. exit-reserve funding address and `unmatched_payments` listing.
9. Webhook secrets are stored in plaintext (`key_encryption_key` is a no-op) — needs a crypto crate.
10. Compose publishes `8080:8080` on all interfaces; no automated TLS.
11. Human gates: UX review, release tag, CHANGELOG sign-off, WooCommerce plugin scope.
