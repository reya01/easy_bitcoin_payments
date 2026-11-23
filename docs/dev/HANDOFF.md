# Handoff — Easy Bitcoin Payments (EBP)

For a new developer or agent. Read this first, then [TECHNICAL.md](TECHNICAL.md).

## Where things stand (22 November 2026)

- **Latest release:** `v0.3.2-rc2` (pre-release, 22 November 2026). Earlier: `v0.1.0-rc1`, `-rc2`,
  `v0.2.0-rc1` to `-rc5`, `v0.2.1-rc1`, `-rc2`, `v0.2.2-rc1`, `v0.3.0-rc1` to `-rc4`, `v0.3.1`, `v0.3.2-rc1`. The release process is in
  [RC_CHECKLIST.md](RC_CHECKLIST.md); [CHANGELOG.md](../../CHANGELOG.md) says what is in each.
- **Experimental.** Signet (play money) is the default; mainnet is opt-in and has been tried end
  to end with small real money (Lightning and Ark payments, a real WooCommerce shop, payouts,
  renewals, restore, a full emergency exit). What is proven on real money and what is not:
  [MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md). How the code behaves in detail:
  [TECHNICAL.md](TECHNICAL.md).
- **What 0.3.0 is, in one paragraph:** EBP runs on **barkd 0.9.0** (every Ark send says `"method":"ark"`);
  every transfer to the owner's wallet is written to a **send journal** first and an unclear one stops all
  transfers until it is settled from the wallet's history or by the owner ("It arrived" / "It did not
  arrive"); the server sends the **whole balance** once it reaches a minimum and keeps nothing (the
  keep-an-amount setting is gone); the emergency withdrawal is **opt-in**, always takes everything, and only
  the 5,000-sat fee money stays; the emergency section on Settings is one plain "next step"; Home's Update card
  has the Console steps as a folded backup; rc3 adds the fixes from the full-repo review (setup only uses a
  wallet it made itself, stricter transfer, sign-in and Lightning checks, no lost shop events, the weekly wallet
  check covering a skipped week). The review and what was fixed or declined is in the project docs
  (`full-review-validation-2026-11-21`).
- **barkd is pinned at 0.9.0** (checksums in both Dockerfiles, `mainnet-compat-check.yml`, the OpenAPI
  snapshot in `docs/barkd-openapi.json`). To move to a newer barkd, follow
  [BARKD_0_9_0_UPGRADE.md](BARKD_0_9_0_UPGRADE.md) (the recipe from the last move). The `bark-canary` workflow
  warns when a newer barkd appears.

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
  touches, `TECHNICAL.md` and, if it needs a live check,
  `DROPLET_TEST_SHEET.md`. Finished plans and dated reports go to [archive/](archive/README.md).

## Read first

1. [README.md](../../README.md): what this is.
2. [TECHNICAL.md](TECHNICAL.md): the technical overview, API, config and security notes.
3. [START_HERE.md](../START_HERE.md): the beginner install; [UPGRADING.md](../UPGRADING.md): updating.
4. [KNOWN_ISSUES.md](../KNOWN_ISSUES.md): open issues and what has not been tested enough.
5. [RC_CHECKLIST.md](RC_CHECKLIST.md): what CI proves, what a person must do, how to cut a release candidate.
6. [CONTRIBUTING.md](../../CONTRIBUTING.md) and [ebp.example.toml](../../ebp.example.toml) (every config key).
7. [SECURITY.md](../../SECURITY.md) and [SILENT_PAYMENTS.md](../SILENT_PAYMENTS.md): reporting, and why payout cannot use silent payment addresses.
8. History, only if you need the why: [archive/](archive/README.md) (the mainnet plan, the Bark 0.9 review, drills and old checklists).

## Architecture decisions and WHY

- **barkd sidecar, not the Bark SDK.** EBP is a thin Rust client of Second’s
  reference Bark wallet daemon over its local REST API. WHY: barkd is built for
  server/webshop use, keeps wallet state and signing in one audited daemon, and
  lets EBP stay a single Rust binary + SQLite. The SDK would couple EBP to
  Bark’s internal Rust API and force wallet state into the payments process.
- **Nothing kept, Custody Level 0.** The server holds only a small “till” and sends the **whole balance** to the
  merchant’s own wallet once it reaches a minimum (hard floors: 1,500 sats for an Ark payout address, 50,000 for an on-chain one; the owner can choose more in Settings, from 19 November 2026). Before that date it kept 1,000 / 1,000,000 sats (`EBP_HOT_WALLET_CAP_SATS` / `EBP_ONCHAIN_SWEEP_CAP_SATS`, both removed on 19 November 2026; if still set they are ignored with a note at start-up) and sent only the excess. Nothing is kept back (the amount-to-keep setting was removed on 20 November 2026); with the opt-in emergency withdrawal on, the 5,000-sat on-chain fee money is protected. “Move my money now” ignores the minimum. WHY: passive Ark receive needs a server-side wallet,
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
- Pinned against: **barkd 0.9.0** (the OpenAPI spec was taken from a running 0.9.0 daemon; the
  response shapes were first measured on 0.8.0 on signet, 1 to 3 November, and the mainnet
  drills of 15 to 16 November confirmed the Lightning, Ark, renewal, exit and restore shapes on
  real money). A real 0.9 send has still to confirm settling an unclear transfer from history.
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
[MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md). In short, on 22 November 2026:

1. **Owner's test of 0.3.2-rc2** on the test server (the quick drills are in that release's cut checklist, in the
   Claude project): update by exact version, Home shows 0.3.2-rc2, payments, transfers and a backup check still read
   back (the database library changed), the admin page loads over https (Caddy changed), one small Ark and one
   Lightning payment. The new update boxes first show on the update after rc2.
   Settling an unclear transfer from the wallet's history is still unconfirmed on a real 0.9 send.
2. **Decisions and answers outstanding:** whether a plain "server error" (HTTP 500) answer to a send should
   count as unclear (today only 502 and 504 do); Second's answers (when the Ark server moves to 0.9, the exact
   upgrade-error text, the size thresholds); a second independent reviewer of the send path; the coin-list note
   that is added on a timeout can corrupt that one line (it fails safe, so the owner is asked).
3. **Owner actions:** GitHub secret scanning and push protection, a `main` ruleset, signing, the README
   comparison-table wording, held Dependabot PRs, and the decisions in `RC_CHECKLIST.md` section 3.
4. **Later:** `update.sh --first-install`, Lightning and silent-payment payouts, refunds, automatic updates. Roadmap
   decided 22 Nov 2026 ([../ROADMAP.md](../ROADMAP.md)): invoices (shareable pay links, plan in [INVOICES.md](INVOICES.md),
   proposed for v0.4), own-domain move (v0.5), then release candidates to 1.0; **Shopify is after 1.0**
   ([SHOPIFY.md](SHOPIFY.md) is a plan).
   Also later, not planned for any release yet: a beginner-friendly guided move from the free `sslip.io` address to
   the owner's own domain (one easy default path, other routes for advanced users). The brainstorm seed is in the
   Claude project (`claude/roadmap-own-domain-brainstorm-2026-11-21.md`); the main constraints are keeping the shop
   working during the move, passkeys being tied to the address, and checking DNS before switching.

## Cold-start commands

```bash
cargo test --all --all-features
cargo run -- store create --name "My Shop" --webhook-url "https://shop.example.com/ebp"
cargo run -- serve
curl -sS http://localhost:8080/api/v1/health
docker compose up -d
docker compose logs --tail=200 ebp
```
