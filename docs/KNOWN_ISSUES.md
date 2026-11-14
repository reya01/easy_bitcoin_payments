# Known issues

## OPEN — beta-hardening follow-ups (2026-11-05)

See `docs/dev/archive/BETA_PLAN.md` for the history of the first hardening pass. Highlights an operator should know:
- **No silent payment (BIP-352) payouts.** Payout addresses must be regular Bitcoin (`bc1…`, or on a real-money server also the older `1…`/`3…`) or Ark addresses; `sp1…` addresses are recognised and refused with an explanation. See [SILENT_PAYMENTS.md](SILENT_PAYMENTS.md).
- **Payment watching re-reads the wallet's whole history every 10 seconds**, because barkd cannot filter by time. Fine for the first hundreds of payments; with many thousands each pass gets slow and payments are noticed later. Needs "skip entries already handled" before a busy shop uses it (plan: [WATCHER_DESIGN_NOTE.md](dev/WATCHER_DESIGN_NOTE.md)). (A missing database index that made this much worse is fixed separately.)
- **On a real-money server, an Ark payout address must belong to Second's Ark server (`ark.second.tech`).** Use Noah (Android) or Arké (iPhone) with the server set to `ark.second.tech`. The setup page warns when an address is on a different Ark server (you can still continue after a danger warning, for example when testing), and automatic transfers and "Move my money now" refuse such an address and say so on Home, so nothing is sent. Only a wrong-server address is caught this way: nothing can check that you control an address.
- **Older `1…`/`3…` payout addresses are accepted on a real-money server** (checked with their checksum), and paid on-chain like `bc1…` ones. Not yet tried with a real transfer.
- **Emergency withdrawal (without the Ark server) is new and untested on a real server.** Settings has it, folded away, with the 100,000-sat and twice-the-fees rule, a fee deposit step, password plus two-step code, and a manual claim. It has only been tested against a mock wallet; it stays switched off on real-money servers until it has been practised on signet (`docs/IF_SOMETHING_LOOKS_WRONG.md` section 9).
- **A payout that keeps failing records a new "failed" line every 5 minutes** (for example an Ark address that belongs to a different Ark server). The sweep itself is right to keep trying and the Home page warns; the activity list just fills up. Planned: record a failure only when its reason changes.
- **The backup file is locked with the admin password** and can contain the wallet. Admin passwords need 16+ characters; use random words rather than a short clever one.
- **DigitalOcean install is verified once, not many times.** On 8 November 2026 the install from `deploy/cloud-init.yaml` on a real SFO3 droplet worked end to end (about 15 minutes to a ready server, 25 to the first payment; the certificate, console banner and wizard all behaved). Later changes have only been through CI. Until a release exists the server builds from source (15–30 min, 2 GB).
- **Tampered-server detection is manual.** A malicious change to the server could redirect payouts or show customers the attacker's address; the payout limit does not cover that. The only defence today is checking your own wallet against shop orders (see docs/OTHER_HOSTS.md). The dashboard now shows a weekly wallet-check card and the WooCommerce plugin warns when Bitcoin orders stop being paid or the server stops answering; both are tripwires, not proof.
- **Managed barkd is verified on signet once.** The 8 November droplet run exercised starting barkd as a child, the wizard, the recovery-words restart and real Lightning and Ark payments. Mainnet behaviour (coin lifetimes, fees) is still unknown; the renewal rule reads those values from the Ark server instead of assuming them. `render.yaml` has still only been compiled and unit-tested.
- **Recovery words are still shown in a browser.** They appear for 60 seconds at a time and are never kept by the page or the server, and the backup check is enforced server-side, but a malicious browser extension or malware on the merchant's device can read them while shown, and screenshots (which often sync to the cloud) are only discouraged, not prevented.
- **Webhook DNS rebinding** is not caught by the address filter (a public name that later resolves to a private address). See docs/dev/archive/SECURITY_REVIEW_2026-11-05.md for this and the other accepted items.
- **Very small amounts cannot be renewed.** The wallet cannot renew coins that add up to less than 330 sats. The server now adds its other coins to reach that amount; if everything on the server is still under 330 sats it skips quietly (no failure warning, no phone alert) and Home shows one yellow line saying when those sats will lapse. The amount at stake is under 330 sats.
- **Renewals show "requested" until the Ark server's round has replaced the coins**, then "renewed" (or "failed", and they are asked for again). On mainnet rounds are hourly, so "requested" can last up to about an hour. Coins already waiting for a round are never asked for twice. Not yet observed on mainnet.
- **Restoring on a new server has been practised once, on play money only.** On 12 November 2026 a second server was restored from the 12 recovery words plus a backup file ([RESTORE_DRILL.md](RESTORE_DRILL.md)): balance, orders, payout address, shop connection, a new test order, "Move my money now" and a fresh backup all worked. It took 30 to 40 minutes, mostly the new server's first start (17 minutes); the restore itself takes about a minute. It has not been practised with real money yet, and it needs the server to restart itself once, which the normal Docker install does.
- **Real Bitcoin (mainnet) is experimental and not yet tried with real money.** A server can be set to it (`EBP_MODE=live`, before setup); the first test with very small amounts is Stage 4 of [dev/MAINNET_PLAN.md](dev/MAINNET_PLAN.md). Keep only small amounts on a real-money server.
- **No continuous wallet backup.** Bark's docs recommend backing up the wallet database after every change. EBP now offers a manual encrypted download (Settings tab → Download a backup) that includes the wallet folder only in managed-barkd mode, but nothing runs automatically, and the real barkd folder layout is still to be confirmed in the signet drill (SIGNET_DRILL.md). The recovery words restore the balance (with the Ark server's help) but not payment history or in-progress exits. See docs/dev/BARK_AUDIT.md.
- **Live signet drill partly done.** The 8 November droplet run covered the wizard, Lightning and Ark payments, pause, and backup download. Still to run on a real server: on-chain payments, settlement over a day or more, and the upgrade backup (see `docs/dev/SIGNET_DRILL.md`). CI covers the logic against a mock barkd only.
- **Release signing: signature checked by hand, `cosign` itself not yet run.** The release workflow ran for `v0.1.0-rc1` and signed `SHA256SUMS`. On 11 November 2026 the signature was checked against the certificate in the bundle: it is valid, and the certificate names this repository's release workflow, the `v0.1.0-rc1` tag and commit `d3197c3`. Still to do: run the `cosign verify-blob` command in [UPGRADING.md](UPGRADING.md) once on a machine with `cosign`, which also checks the certificate chain and the public log entry. Protecting `v*` tags and the account that publishes releases matters as much as the signature.
- **Pre-upgrade backups share the disk** with the database: they protect against a bad upgrade, not disk loss (see RECIPE_BACKUP.md).
- **The Bitcoin price comes from five free public websites** (mempool.space, blockchain.info, CoinGecko, Kraken, Coinbase), asked every ten minutes. One or several being down is fine; all of them failing keeps the last price for up to a day, then dollar amounts disappear and orders priced in a currency are refused (orders priced in sats are unaffected). Whether the free limits hold up on a shared cheap-server address is not yet proven. Orders can be priced in USD, EUR, GBP, CHF, CAD, AUD or JPY only.
- **Phone alerts are untested on a real phone.** The server can send problems to an ntfy topic (admin → Home); CI covers the logic and the address checks, but nobody has scanned the QR code with the real ntfy app yet. Alerts depend on the EBP server, so they cannot warn you if the server itself is replaced by something hostile; that is what the shop-side tripwire and the weekly wallet check are for.
- **Log retention is fixed.** The cleanup windows are constants in `src/retention.rs`, not settings. The database file does not shrink after a cleanup (freed space is reused); run `VACUUM` offline if you need the file smaller.
- **Admin login limits.** One admin account for the (single) store; two-step sign-in uses an authenticator app (TOTP); passkeys are planned for later; the setup wizard does not offer two-step sign-in, the admin page does at first sign-in; no email-based reset (by design: a forgotten password is reset in the server's Console with `cd /opt/ebp && docker compose exec ebp ebp admin set-password`). Sessions last 7 days, 12 hours idle. The admin page has only been tested in a desktop browser (against a stub server), not on a phone or a live server.
- **Beginner path is only partly proven.** The DigitalOcean route needs no terminal, but it has only been rehearsed on a clean Ubuntu machine in CI, not on a real droplet. Every other install route (your own computer or server, Render) needs a terminal or is untested. See [BEGINNER_ROADMAP.md](dev/archive/BEGINNER_ROADMAP.md).
- **No Shopify support.** WooCommerce is the supported shop, and the first-class one. Shopify is planned for a much later release, after WooCommerce is thoroughly tested; the research is in [SHOPIFY.md](dev/SHOPIFY.md).
- **Render is not tested on real Render.** The blueprint is checked in CI for safe settings (paid plan, disk, manual deploys) only. See [RENDER.md](OTHER_HOSTS.md).
- **The release image build is slow.** The multi-architecture image takes well over half an hour in CI (the arm64 half runs under emulation). A real release will take about that long to finish.
- The hardening branch (PR #1) was authored without a local compiler; CI (fmt, clippy -D warnings, tests, release build) is the verification. Keep it green.
- Late/unmatched payments land in the `unmatched_payments` table with no dashboard UI yet — use `GET /api/v1/unmatched-payments` (store API key) or query the table directly
- Webhooks are delivered from a durable outbox (`webhook_deliveries`); failed rows (after 6 attempts) stay for inspection with no re-send UI yet. Webhook URL validation is textual and does not stop DNS rebinding — restrict container egress in production.
- Webhook secrets are encrypted at rest (R10, done). Residual: with the auto-created `ebp.key` file the key sits in the same volume as the database, so only set `EBP_KEY_ENCRYPTION_KEY` from outside the volume gives real separation; the in-memory key and decrypted secrets exist in process memory while serving.
- On-chain receive detection depends on an UNPINNED `/history` shape (the deprecated `/wallet/history` is no longer used). The rail is therefore OFF by default (`EBP_ONCHAIN_RAIL_ENABLED`); do not enable it for real funds before a live signet drill.

## RESOLVED & CONFIRMED — VPS aarch64 container restart-loop (~60s, 22h duration)

**Status:** root cause proven 2026-11-03, fix committed and **confirmed on the
VPS** the same night: the tester rebuilt (`git pull` → `docker compose build`
passing the new smoke test) and ebp now boots, logs "Easy Bitcoin Payments
listening", and stays up.

**Root cause: the deployed image contained the dependency-cache STUB binary,
not ebp.** The Dockerfile's dep-cache step builds `fn main() {}` as a
placeholder; the real build replaces it. The tester's arm64 image shipped the
stub instead: `ebp --help` inside it printed NOTHING and exited 0 (a real
binary prints the USAGE block). An empty main exits instantly and cleanly —
hence `Restarting (0)` on a ~60s cadence (Docker's restart-backoff cap) for
22 hours, with no logs, OOM=false, and no change to barkd (healthy throughout
— its image downloads a prebuilt binary, so the trap can't touch it).

**Mechanism (proven locally 2026-11-03):** cargo skips relinking when source
mtimes are OLDER than the cached stub binary. git clone/pull stamps sources
with the clone time and Docker COPY preserves those old mtimes, so on every
VPS rebuild the real `cargo build` was a ~1s no-op and the stub shipped. A
`--no-cache` rebuild with the new smoke test reproduced the trap (build
failed exactly as designed); `find src -type f -exec touch {} +` before the
real build forces the recompile (proven: 7m real recompile → real 8.1MB
binary).

**Evidence chain (all 2026-11-03):**
- `docker inspect`: exit 0, OOM=false, RestartCount 1360, Started/Finished
  297ms apart — instant clean death, ~58s apart ≈ backoff cap.
- `docker events` over 26h: bare `die` events, `execDuration=0 exitCode=0`,
  ~60.3s apart, NO kill/stop before any die — the loop was self-exit +
  restart policy, not an external signaler.
- `docker run --rm easy_bitcoin_payments-ebp ebp --help` → silent, exit 0
  (stub); local build of the exact image-time commit (0e9fd88) boots and
  serves fine on x86_64 → source was never the problem.
- Exit-code matrix eliminated every earlier suspect: config/bind/db error=1,
  panic=101, OOM=137, exec failure=126/127 — none can print 0.

**History correction (recorded so it isn't repeated):** the 2026-11-02
commits (90a8a95, b264da9) attributed the loop to "crashed on 'no such
table: settings', found via tester's first-run". That was code inspection
presented as an observation — no error text was ever captured, `settings` is
read from wizard routes (not boot), and the first image was already a stub
that cannot query anything. The `"/ping never answers"` symptom was also
trivially true: EBP has no `/ping` route (the health endpoint is
`/api/v1/health`. The 0004 migration registration fix remains correct as a
real latent bug (it would have broken the setup wizard on fresh databases)
but was never the cause of the loop.

**Guards now in the Dockerfile (b0fc908, b698768):**
1. `RUN find src -type f -exec touch {} + && cargo build --release` — mtime
   refresh forces the crate recompile; dep cache untouched.
2. The dep-cache step deletes the stub binary after warming (`rm -f
   target/release/ebp`) — a skipped real build leaves no binary at all.
3. `RUN timeout 10 ./target/release/ebp --help 2>&1 | grep -qi usage` — the
   build refuses to ship anything that can't print a usage block.
4. The barkd image checksum-pins its prebuilt binary.

## Added with the open-items PR (2026-11-05)
- None of the barkd paths changed in PRs #3 and this one (Lightning invoice, Ark send, VTXO refresh, partial offboard, wallet create with `ark_server`, `/history`) has run against a live barkd; drill each on signet before real funds.
- The QR encoder is original code verified with OpenCV and one Chromium render, not a phone scan; scan a real invoice with two wallets before launch.
- History is still read in full each tick (R5).

## Review notes on the open-items PR (never deployed, so no migration paths)
- **On-chain payments are confirmation-gated, and unverified live.** With the rail on, an on-chain payment is credited only when `/onchain/transactions` shows it mined at depth `EBP_ONCHAIN_MIN_CONFIRMATIONS` (default 1); mempool payments show "waiting for confirmation" and are never credited (RBF-safe). The deadline is judged by when EBP first SAW the payment, and the invoice is held open (max 24h past expiry) while it waits. Caveats: a reorg deeper than the chosen depth can still undo a payment (raise the setting for large orders); a replaced or never-confirming payment keeps the order pending until the 24h hold ends; the live `/onchain/transactions` shape has not been exercised against signet yet, and the rail stays OFF by default until that drill passes.
- **The shop's API key is one key for everything a shop does** (create, read and cancel payment requests, read their status). It cannot change the notification address, replace itself, resend notifications, mark payments handled, or read the payout address, security activity, notification log or late-payment list, and it is limited in request size and rate. A hacked shop can still make fake or cheap payment requests within those limits. See [SHOP_COMPROMISE.md](dev/SHOP_COMPROMISE.md).
- **`rotate-key` shows the new key once.** If the response is lost, run `ebp store rotate-key` on the server (inside Docker: `docker compose exec ebp ebp store rotate-key`) to mint another. It is audit-logged.
- **The setup code is shown in the server log.** Detached `docker compose up -d` hides it; the recipe tells the merchant to run `docker compose logs ebp`. A platform that injects `EBP_SETUP_TOKEN` (one-click installers) skips this.
- **Defensive parsing left from before live captures.** A wallet answer that is valid JSON but the wrong shape (for example `null`) is read leniently and the health check can still say the wallet is reachable (with "ASP connection unknown"); answers that cannot be read at all are reported as down. The end-to-end test `garbage from the wallet` covers both and checks no payment is credited wrongly or lost. `barkd.rs` still accepts several alternative field names and envelope shapes (`flex::*`, history/vtxo fallbacks). Now that the OpenAPI is pinned and a live history capture exists, trim them after the signet drill so unexpected shapes fail loudly.

## Two-step sign-in and other limits found in review (2026-11-08)
- **Someone who knows your password can lock you out of two-step for a while.** Five wrong codes pause code tries for ten minutes, and an attacker who has the password can repeat that. They still cannot get in; you wait, or run `ebp admin reset-2fa` on the server. A longer pause after repeated lockouts is planned.
- **A code used to sign in can approve a payout change for about 90 seconds** (it must be a different code for the next sign-in). Acceptable for now; strict one-time use for protected actions is planned.
- **The shop address-lock only means something if the server is reached through your proxy.** The DigitalOcean install already does this right (Caddy in front, EBP listening only on the server itself, "trust proxy headers" on). If you change that, do not leave port 8080 reachable around the proxy; otherwise a request can claim any address. The dashboard shows the last refused address so a mistake is easy to fix.
- **Alert summaries can very rarely count a payment twice** (one that lands in the same second a summary is cut). Counts only, never amounts.
- **A session (the signed-in browser) can change the shop-address lock, pause payments and change the alert server without a code.** That matches the existing webhook and key actions; a code for these is being considered.

## Ark coins in your own wallet also expire (your wallet app must renew them)

Coins held over Ark have a limited lifetime: **28 days** on the Ark test server
(Second's server; the real server decides its own value). The payment server
renews its own coins automatically. The coins it sends you over Ark **keep the
same expiry date**, so renewing them is your own wallet app's job, not EBP's.
Good Ark wallet apps do this automatically in the background; EBP cannot see or
change what your app does. Once a coin expires, the Ark server can take it back,
and getting your money out then needs the server's cooperation.

- Check that the wallet app you use renews coins by itself (its settings or its
  documentation will say). If you cannot find out, treat it as not renewing.
- If you want money to sit untouched for months, or you are unsure, use an
  **on-chain** payout address: bitcoin on-chain never expires.

## On-chain payouts reuse one address (privacy trade-off)

An on-chain payout address is a single fixed address: every withdrawal goes
to it. That keeps setup simple for beginners, but anyone watching the chain can
link all your payouts together. Ark payouts do not have this problem. This is a
deliberate, known trade-off. Planned improvement: accept a Silent Payments
address (BIP 352) as the on-chain payout destination, so each withdrawal lands
on a fresh address without changing the simple "paste one address" setup. That
waits on wallet and ecosystem support.
