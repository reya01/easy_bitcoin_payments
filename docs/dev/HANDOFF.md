# Handoff — Easy Bitcoin Payments (EBP)

## Update 2026-11-11 (read this first)

`v0.1.0-rc1` is published (signet only). The next piece of work is the move to mainnet with
small real money, and the agreed plan is [MAINNET_PLAN.md](MAINNET_PLAN.md): restore from
the 12 words, mainnet wallet and addresses, three renewal changes, a docs sweep, then a
mainnet test sheet. Follow it in order. barkd stays on 0.8.0 during alpha.

## Update 2026-11-06

Since the notes below were written, these were added and are on `main` with CI
green: phone alerts (ntfy), a working WooCommerce plugin (real WordPress +
WooCommerce end-to-end test in CI), a return-to-shop link, the DigitalOcean
no-terminal install (rehearsed on clean Ubuntu in CI), hosting docs
(`OTHER_HOSTS.md`, `OTHER_HOSTS.md`), a weekly own-wallet check and a shop-side
tripwire, a security pass (alerts address guard, order-total mismatch guard),
an upgrade and backup-restore test in CI, a hung-barkd fix for the health
endpoint, and a release dry run that packages everything without publishing.
Still **not** done: the live signet drill, a real DigitalOcean run, Render on
real Render, Shopify (`SHOPIFY.md` is a plan only), the first release tag.
The checklist for getting from here to a release candidate is
[RC_CHECKLIST.md](RC_CHECKLIST.md). Test counts change often; CI is the source
of truth, not the number below.

**Status:** Phase 1 code-complete (2026-11-01), **EXPERIMENTAL — not for real funds**.
Tests: see CI. All client-facing barkd response shapes are
live-verified against **barkd 0.8.0 on signet** (Lightning side; the on-chain
receive history entry shape is still unpinned — crediting is fail-closed) and
pinned in
[`src/wallet/barkd/live_shapes.rs`](../../src/wallet/barkd/live_shapes.rs) with shape tests.
Remaining `VERIFY` markers are limited to request/edge shapes the pinned OpenAPI
does not schema.

**Resolved 2026-11-03:** the tester's AWS t4g (aarch64) VPS restart-loop was
the Dockerfile's dependency-cache placeholder binary shipping instead of the
server (cargo mtime-skip trap; fix + build smoke test in place; rebuilt and
serving on the VPS). Full story: [docs/KNOWN_ISSUES.md](../KNOWN_ISSUES.md).

## Read first

1. [README.md](../../README.md) — what this is, quickstart, API, config.
2. [docs/dev/PHASE1_PLAN.md](PHASE1_PLAN.md) — Phase 1 tasks and close-out status.
3. [docs/dev/PHASE0_SPIKE_PLAN.md](PHASE0_SPIKE_PLAN.md) — Phase 0 spike questions/log.
4. [docs/START_HERE.md](../START_HERE.md) — the beginner install.
5. [docs/KNOWN_ISSUES.md](../KNOWN_ISSUES.md) — open issues and debug procedures.
6. [docs/START_HERE.md](../START_HERE.md) — beginner AWS + iPhone walkthrough.
7. [CONTRIBUTING.md](../../CONTRIBUTING.md) — build, test, contribution rules.
8. [ebp.example.toml](../../ebp.example.toml) — every config key.
9. [RC_CHECKLIST.md](RC_CHECKLIST.md) — what CI proves, what a person must still do, how to cut a release candidate.
10. [BETA_PLAN.md](BETA_PLAN.md) — history of the first hardening pass (not a to-do list).
11. [SECURITY.md](../../SECURITY.md) and [SILENT_PAYMENTS.md](../SILENT_PAYMENTS.md) — reporting, and why payout cannot use silent payment addresses.

## Architecture decisions and WHY

- **barkd sidecar, not the Bark SDK.** EBP is a thin Rust client of Second’s
  reference Bark wallet daemon over its local REST API. WHY: barkd is built for
  server/webshop use, keeps wallet state and signing in one audited daemon, and
  lets EBP stay a single Rust binary + SQLite. The SDK would couple EBP to
  Bark’s internal Rust API and force wallet state into the payments process.
- **Capped till, Custody Level 0.** The server holds only a capped hot “till”
  (`EBP_HOT_WALLET_CAP_SATS`, default 1,000 sats for an Ark payout address; `EBP_ONCHAIN_SWEEP_CAP_SATS`, default 1,000,000, for an on-chain one) and sweeps excess to the
  merchant’s own wallet. WHY: passive Ark receive needs a server-side wallet,
  but the honest custody story is “minimize, cap, sweep home, document exactly.”
  (The on-chain exit-reserve warning was removed: it asked for more money on the server, against that principle.)
- **No upstream assumptions.** EBP works with Bark/barkd/Arké as they exist
  today. WHY: the spec explicitly assumes no upstream contributions and no
  upstream roadmap; missing capabilities are designed around inside EBP.
- **VPS-first, no Umbrel.** Supported deploy is Docker on a VPS with public IP,
  DNS, and TLS. WHY: the payment server must be reachable by stores and
  customers; home/Tor-only hosting is out of scope for v1.0.
- **Lightning via the ASP gateway.** Lightning receive is on by default and
  uses the Ark Service Provider’s Lightning gateway. WHY: Lightning wallets
  vastly outnumber Ark wallets today; the gateway is the only LN receive rail
  available without adding a separate node/swap provider. Fees are the payer’s
  and are shown when the live fee table is available.
- **Refunds out of scope for v1.0.** Bitcoin payments are final; the checkout
  page says so. WHY: refunds require a separate custody/approval model and are
  not needed to prove the payment loop.

## Verified live shapes

- Pointer: [`src/wallet/barkd/live_shapes.rs`](../../src/wallet/barkd/live_shapes.rs).
- Pinned against: **barkd 0.8.0, signet** (responses 2026-11-01; the full
  `/wallet/history` page 2026-11-03).
- Covered: wallet mnemonic (`{"mnemonic": "…"}` under `--expose-mnemonic`),
  wallet connected (`{"connected": true}`), Lightning receive fee body, VTXO
  list empty shape (`[]`), and the other client-facing response bodies used by
  the watcher/settlement/setup paths — plus the REAL history-entry shape:
  receives carry `subsystem.kind`, `received_on[]` (destination objects:
  LN = type "invoice" + the BOLT11 string; amount in `amount_sat`),
  `metadata.htlc_vtxos` (dedupe key), `time.completed_at` (RFC 3339), and
  `status` ("successful" required before crediting). There is NO
  entry-level address/amount/id.
- **Crediting is FAIL-CLOSED**: an invoice is credited only on live-verified
  receive evidence (`subsystem.kind = "receive"`, non-empty `received_on`,
  or an explicit `direction: "receive"`). Round/send/board movements and
  signal-less entries NEVER credit; a refused payment-looking entry logs a
  loud pin-me warning. STILL UNPINNED: the on-chain receive entry's exact
  shape — watch for that warning in any on-chain drill.
- Drift check: [`scripts/check_barkd_drift.sh`](../../scripts/check_barkd_drift.sh)
  verifies every `PATH_*` constant against
  [`docs/barkd-openapi.json`](../barkd-openapi.json) in CI (and fails loudly if
  the scanner itself matches nothing).

## Hardening + ops (added 2026-11-03)

- **Build guards**: the Dockerfile's dep-cache step (which builds a throwaway
  stub binary) deletes the stub after warming, refreshes source mtimes before
  the real build, and runs a usage smoke test — the stub-ship failure class
  that caused the 22h restart-loop cannot ship again
  ([docs/KNOWN_ISSUES.md](../KNOWN_ISSUES.md) has the full forensic record).
- **CI**: builds BOTH release architectures (amd64 + arm64 via QEMU),
  shellchecks every shipped script, clippy `-D warnings`, drift check,
  tests. Verify CI is GREEN after every push — it once went red for a day
  unnoticed.
- **Backups**: the admin page's Settings → Download a backup gives one
  encrypted file (database, encryption key, and in managed mode the wallet
  folder). Nothing is automatic yet. The paper recovery words remain the
  only human-verifiable wallet backup.
- **Schema ownership**: the `settings` table (key/value/`updated_at NOT NULL`)
  is owned by `src/settings.rs`; `db.rs`'s `get_setting`/`set_setting` are
  aliases. Migration 0003 creates the table; 0004's `IF NOT EXISTS` copy
  no-ops. Never add a second settings implementation.
- **Wiring test**: `tests/routes_wiring.rs` serves the real composed router
  and asserts the checkout page's fetched routes (`/i/{id}/json` once slipped
  through unwired), the setup gate (503 pre-setup), and API auth (401).

## Drills performed and what they proved

- **Ark sweep drill.** Sent from the till to an Arké signet address; proved the
  settlement engine’s Ark-send path, cap trigger, and destination parsing
  (`ark1…` / `tark1…`) work against live barkd.
- **Lightning checkout drill.** Created a checkout invoice with LN enabled,
  paid the BOLT11 from a signet LN wallet, and watched the claim land as a VTXO.
  Proved the ASP-gateway receive path, BOLT11 attribution in
  `find_pending_by_address`, and the payer-fee behavior (5,020 sent for a
  5,000-sat invoice; payee receives the full invoice amount).
- **Byte Store purchase drill.** End-to-end store purchase against the hosted
  checkout; proved invoice creation, checkout polling, payment attribution,
  state transition, and signed webhook emission.

## Phase plan

- **Phase 0:** [docs/dev/PHASE0_SPIKE_PLAN.md](PHASE0_SPIKE_PLAN.md) — Bark/barkd
  spike questions, commands, and observed outputs.
- **Phase 1:** [docs/dev/PHASE1_PLAN.md](PHASE1_PLAN.md) — barkd integration,
  watcher, expiry queue, settlement engine, setup wizard, live shape pinning.

## What remains

The authoritative list is [RC_CHECKLIST.md](RC_CHECKLIST.md) (sections 2 and 3).
In short:

1. **Things only a person can do**: the live signet drill
   ([SIGNET_DRILL.md](SIGNET_DRILL.md)), a real DigitalOcean run
   ([VPS_DIGITALOCEAN.md](../START_HERE.md)), WooCommerce on a real shop host,
   phone alerts on a real phone, a backup restore on a second machine, and a
   newcomer following [START_HERE.md](../START_HERE.md) without help.
2. **Decisions**: the public GitHub name (it appears in several files; change it
   before the first tag), turning on private vulnerability reporting, protecting
   `v*` tags, and deleting stale branches (never merge `mut-test`).
3. **The release cut.** A human chooses and confirms the tag. Start with
   `v0.1.0-rc1` (a pre-release; play money only). Pushing a `v*` tag makes
   `.github/workflows/release.yml` build the binaries, `SHA256SUMS`, the plugin
   zip and the multi-arch image. Signing has not been exercised yet. A manual
   run of the workflow is a dry run that publishes nothing.
4. **Still unpinned**: the on-chain receive history entry shape. Crediting is
   fail-closed, and the on-chain rail is off by default until the signet drill
   has been run.
5. **Not built**: Shopify ([SHOPIFY.md](SHOPIFY.md) is a plan), Lightning
   payouts, silent payment payouts, refunds, automatic updates.

## Cold-start commands

```bash
cargo test --all --all-features
cargo run -- store create --name "My Shop" --webhook-url "https://shop.example.com/ebp"
cargo run -- serve
curl -sS http://localhost:8080/api/v1/health
docker compose up -d
docker compose logs --tail=200 ebp
```
