# Handoff — Easy Bitcoin Payments (EBP)

For a new developer or agent. Read this first, then [TECHNICAL.md](TECHNICAL.md).

## Where things stand (19 November 2026)

- **Latest release:** `v0.2.2-rc1` (pre-release). Earlier candidates: `v0.1.0-rc1`, `-rc2`,
  `v0.2.0-rc1` to `-rc5`, `v0.2.1-rc1`, `-rc2`. The release process is in
  [RC_CHECKLIST.md](RC_CHECKLIST.md); [CHANGELOG.md](../../CHANGELOG.md) says what is in each.
- **Experimental.** Signet (play money) is the default; mainnet is opt-in and has been tried end
  to end with small real money (Lightning and Ark payments, a real WooCommerce shop, payouts,
  renewals, restore, a full emergency exit). What is proven on real money and what is not:
  [MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md). How the real-money code paths behave:
  [MAINNET_CODE_PATHS.md](MAINNET_CODE_PATHS.md).
- **0.2.2-rc1 in one paragraph:** the server sends the **whole balance** to the owner's wallet
  once it reaches a minimum (floors 1,500 sats Ark, 50,000 on-chain) and keeps nothing; the
  emergency withdrawal is **opt-in** with 5,000 sats of protected fee money and a **send back
  what is above 5,000** button; **passkeys** (beta) for sign-in and protected actions; a transfer
  never sends coins waiting in a renewal round; a locked-down container; checkout is Lightning
  and Ark only. It is being tested on the owner's test server (it was updated there with the Home-tab
  update, the update helper's first real use); the run order is the "Round 3" section of [DROPLET_TEST_SHEET.md](DROPLET_TEST_SHEET.md).
- **barkd stays pinned at 0.8.0.** **v0.3 means running Bark 0.9** (its dust-output change affects
  sweeps and exits): [BARK_0_9_REVIEW.md](BARK_0_9_REVIEW.md).

## Working rules

- **Money safety comes first.** When in doubt, refuse and say why in plain words; nothing may move
  funds without the admin password and a second step.
- **Beginner-friendly, no jargon, no terminal.** The owner works from a phone. Every screen and
  message must make sense to a first-time shop owner.
- **The owner owns** tags, releases, GitHub settings, the DigitalOcean test server and anything
  that spends real money. Agents prepare PRs and docs; they do not tag or release.
- **CI is the compiler.** There is no local `cargo build` in the usual agent environment; push a
  branch and read the checks. Browser page tests (`tests/e2e/`) and `rustfmt` can run locally.
- **Never commit real Bitcoin addresses** (treat them like personal data); use dummy ones.
- **Docs move with the code.** A change to behavior updates `CHANGELOG.md`, the user guide it
  touches, `TECHNICAL.md`, `MAINNET_CODE_PATHS.md` and, if it needs a live check,
  `DROPLET_TEST_SHEET.md`. Finished plans and dated reports go to [archive/](archive/README.md).

## Read first

1. [README.md](../../README.md): what this is.
2. [TECHNICAL.md](TECHNICAL.md): the technical overview, API, config and security notes.
3. [START_HERE.md](../START_HERE.md): the beginner install; [UPGRADING.md](../UPGRADING.md): updating.
4. [KNOWN_ISSUES.md](../KNOWN_ISSUES.md): open issues and what has not been tested enough.
5. [RC_CHECKLIST.md](RC_CHECKLIST.md): what CI proves, what a person must do, how to cut a release candidate.
6. [CONTRIBUTING.md](../../CONTRIBUTING.md) and [ebp.example.toml](../../ebp.example.toml) (every config key).
7. [SECURITY.md](../../SECURITY.md) and [SILENT_PAYMENTS.md](../SILENT_PAYMENTS.md): reporting, and why payout cannot use silent payment addresses.
8. History, only if you need the why: [MAINNET_PLAN.md](MAINNET_PLAN.md) and [archive/](archive/README.md).

## Architecture decisions and WHY

- **barkd sidecar, not the Bark SDK.** EBP is a thin Rust client of Second’s
  reference Bark wallet daemon over its local REST API. WHY: barkd is built for
  server/webshop use, keeps wallet state and signing in one audited daemon, and
  lets EBP stay a single Rust binary + SQLite. The SDK would couple EBP to
  Bark’s internal Rust API and force wallet state into the payments process.
- **Nothing kept, Custody Level 0.** The server holds only a small “till” and sends the **whole balance** to the
  merchant’s own wallet once it reaches a minimum (hard floors: 1,500 sats for an Ark payout address, 50,000 for an on-chain one; the owner can choose more in Settings, from 19 November 2026). Before that date it kept 1,000 / 1,000,000 sats (`EBP_HOT_WALLET_CAP_SATS` / `EBP_ONCHAIN_SWEEP_CAP_SATS`, both removed on 19 November 2026; if still set they are ignored with a note at start-up) and sent only the excess. Money is kept back only if the owner turns on the opt-in emergency withdrawal and chooses an amount; the 5,000-sat on-chain fee money is then protected. “Move my money now” ignores the minimum. WHY: passive Ark receive needs a server-side wallet,
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
- Pinned against: **barkd 0.8.0** (signet responses 2026-11-01 and 2026-11-03; the
  mainnet drills of 15 to 16 November confirmed the Lightning, Ark, renewal, exit and
  restore shapes on real money).
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
  loud pin-me warning. (EBP no longer takes regular on-chain payments, so the
  on-chain receive entry shape no longer matters for crediting.)
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
- **CI**: builds the release image for amd64 only (arm64 dropped on 17 Nov 2026 as too slow; planned back at v1.0),
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

## What is open

The authoritative lists are [KNOWN_ISSUES.md](../KNOWN_ISSUES.md) and section 3 of
[MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md). In short, on 19 November 2026:

1. **Owner's test of 0.2.2-rc1:** the Home-tab update, a whole-balance Ark send with nothing
   kept (does the wallet take a fee on an exact whole balance?), the minimums, the emergency
   withdrawal opt-in and its send-back, passkeys. Problems become 0.2.2-rc2.
2. **Move to Bark 0.9 before v0.3-rc1**, with an explicit `"method":"ark"` on sends, a guardrail
   that checks the net amount before an on-chain sweep, and a re-drill of the emergency exit on
   0.9 with small coins. Waiting on answers from Second (when the Ark server moves to 0.9, the
   exact figures and error text in the review).
3. **Owner actions:** GitHub secret scanning and push protection, a `main` ruleset, the README
   comparison-table wording, and the decisions in `RC_CHECKLIST.md` section 3.
4. **Later:** `update.sh --first-install`, a webhook DNS-rebinding guard, `--locked` in the
   Dockerfile, Shopify ([SHOPIFY.md](SHOPIFY.md) is a plan), Lightning and silent-payment
   payouts, refunds, automatic updates.

## Cold-start commands

```bash
cargo test --all --all-features
cargo run -- store create --name "My Shop" --webhook-url "https://shop.example.com/ebp"
cargo run -- serve
curl -sS http://localhost:8080/api/v1/health
docker compose up -d
docker compose logs --tail=200 ebp
```
