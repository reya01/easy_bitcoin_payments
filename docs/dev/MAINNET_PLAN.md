# Mainnet migration and wallet-restore plan

Revised 11 November 2026 (fourth revision) against `main` at `d3197c3` (`v0.1.0-rc1`). This is a plan, agreed with the owner on 11 November 2026. Nothing in it is built yet except the sweep threshold of 1,000 sats (PR #151). Whoever does the work follows it in order and stops to report when reality differs from it.

**Goal:** the same features that work on signet today, working on Bitcoin mainnet with very small amounts of real money, plus a documented and tested way to get everything back after losing the server. No other new features and no new dependencies.

## Owner decisions already made

- **Sweep threshold is 1,000 sats** for an Ark payout address, on signet and mainnet (was 20,000). Not near zero: that would create many tiny coins. PR #151.
- **Restore from the 12 words will be built** before any real money (Part B). Sonnet does all coding.
- **barkd stays on 0.8.0** during alpha. Updating to the latest Bark happens when moving to beta, as its own piece of work.
- **Legacy mainnet payout addresses (`1…`, `3…`) will be accepted** if barkd can pay to them (Stage 2 PR C).
- **Renewal (approved 11 November):** build the three changes in Stage 2 PR D (skip amounts below the 330-sat minimum, show "requested" until the round is done, renew earlier if rounds are very slow), then observe the rest on mainnet with small funds.
- Signet stays the default. Mainnet is opt-in through `EBP_MODE=live`.

## Rules for whoever executes this

- Biggest priority is not losing funds. If a result differs from what this plan expects, stop and report. Do not improvise a fix on mainnet.
- Never ask the owner for recovery words, the admin password, the API key or the signing secret.
- No tags without the owner's approval. The owner publishes releases in GitHub's web UI.
- Every code change goes through a PR with all 7 CI checks green. There is no local `cargo` build in the cloud session; CI is the test run.
- Every code PR updates the docs it affects and `CHANGELOG.md` in the same PR (see "Docs" below).
- An existing signet server cannot be converted. A barkd wallet belongs to one network, so mainnet needs a fresh server and new recovery words.

## Order of work

1. Stage 1: owner tests the 1,000 threshold on signet (PR #151).
2. Part B: restore from the 12 words, built and drilled on signet.
3. Stage 2: mainnet code changes.
4. Stage 3: release candidate.
5. Stage 4: mainnet test with small real money.
6. Stage 5: record what mainnet showed.

## Facts from the Bark docs this plan relies on

| Fact | Value | Source |
|---|---|---|
| Mainnet Ark server | `https://ark.second.tech` | connection-details, bark-cli/mainnet |
| Mainnet Esplora | `https://mempool.second.tech/api` | connection-details |
| Mainnet wallet create (CLI) | `bark create --mainnet --ark https://ark.second.tech` | bark-cli/mainnet |
| Same thing through barkd | `POST /wallet/create` with `network: "mainnet"`, `ark_server`; `chain_source` optional because mainnet has a default | pinned `docs/barkd-openapi.json` |
| Restore through barkd | the same call with `mnemonic` set (also optional `birthday_height`, `gap_limit`) | pinned `docs/barkd-openapi.json` |
| What the words recover | the Ark balance (with the Ark server's cooperation) and the on-chain balance; **not** payment history or in-progress exits | bark-cli/mainnet, backups |
| What a wallet-folder backup adds | history and in-progress exits, without the server's help | backups |
| Coin lifetime | 28 days for boards and refreshes; 1 to 3 days for coins from Lightning receives | learn/lifetime |
| Refresh fee | free in the last 2 days before expiry, then 0.2% / 0.4% / 0.5% | second.tech/pricing, learn/fees |
| Lightning receive, Ark send, Ark receive | 0% | second.tech/pricing |
| Rounds | much less often on mainnet than signet; a refresh waits for the next round; exact interval not documented | bark-cli/mainnet |
| Second's advice for mainnet | start with small amounts; save the seed phrase; back up the wallet data folder continuously; only store what you can afford to lose | bark-cli/mainnet |
| Mainnet support | chat.second.tech, contact Neil (982572) | bark-cli/mainnet |

Not documented, so they must be measured in Stage 4 or asked of Second's support: the mainnet round interval, the smallest Ark send (dust limit), and whether mainnet sets `ln_receive_anti_dos_required`.

### How this plan follows Second's "Test on mainnet" page

| Second's page says | Where this plan does it |
|---|---|
| Create the wallet with `--mainnet --ark https://ark.second.tech` | Stage 2 PR A: wizard sends `network: mainnet` and that server by default |
| Start with small amounts | Stage 4 money limit and `EBP_MAX_INVOICE_SATS` |
| Fund first with a small Lightning invoice | Stage 4 step 5 |
| Save the seed phrase | existing wizard step, unchanged |
| Continuous backups of the data folder | **Gap.** EBP has a manual encrypted backup download, not a continuous one (`bark-backupd` is not packaged). Stays a documented known issue; Part B makes the manual backup plus the words a tested recovery path |
| The seed cannot restore history or in-progress exits | Part B: history comes from the EBP backup file, and the docs say exits are not covered |
| Refresh coins before they expire; rounds are slower | Stage 2 PR D, then observed on mainnet with small funds |
| Only store what you can afford to lose | 1,000-sat threshold, real-money warning line, test money limit |

## Stage 1: test the 1,000 threshold on signet (owner)

PR #151 changes the default. To test before it is merged and released, set `EBP_HOT_WALLET_CAP_SATS=1000` in `/opt/ebp/.env` on the signet droplet and restart with `./scripts/start.sh`.

How the threshold behaves: the server keeps 1,000 sats and sends on everything above that (`src/wallet/settlement.rs:375`). So once the server holds 1,000, every later payment is sent on in full, however small.

1. Pay a 2,000-sat order. Expect about 1,000 sent to the payout wallet within about 10 minutes and about 1,000 left on the server.
2. Pay a 200-sat order, then a 500-sat order. Expect each sent on. 200 is below the wallet's 330-sat dust limit and 500 is above it, so this shows whether a small Ark send is accepted and how many small coins the payout wallet ends up with.
3. Record any failed transfer row or alert, and what `bark vtxos` shows in the payout wallet.

**If small sends fail or the pile of small coins is a problem:** stop and report. The fix would be a change to the sweep rule (for example only sending once the excess itself is above a minimum), which is a separate owner decision, not part of this plan.

## Part B: restore after losing the server (words plus backup file)

### What must be true at the end

A beginner who loses the server can, with the 12 words and their latest `.ebpbackup` file, get back the money, the order history, the settings and the shop connection, by following one page of steps. It has been done once for real on signet before mainnet.

### What each thing restores

| Thing | Restores | Does not restore |
|---|---|---|
| 12 recovery words | the money still in the server's wallet | order history, settings, in-progress exits |
| `.ebpbackup` file (opened with the admin password from when it was downloaded) | order history, settings, payout address, admin password, shop API key and signing secret, the server's encryption key | money that moved after the download; two-step sign-in (off after a restore, by design) |
| WooCommerce itself | its own orders and their paid status, even ones made after the last backup | nothing on the payment server |

**Rule for the docs and the code:** the money always comes from the words. The wallet folder inside the backup file is a snapshot and may be out of date, so the restore never uses it when the words are available.

### Build (Sonnet), smallest version that meets the goal

1. **Wizard: "Restore a wallet I already have".** On a fresh server the wallet step offers two choices: make a new wallet (today's path) or restore. Restore asks for the 12 words and calls the existing `create_till_wallet` (`src/routes/setup.rs:777`) with `mnemonic` added to the same request. No new dependency: barkd checks the words; EBP shows barkd's refusal in plain language.
2. **Handling the words.** Same care as when they are shown today: HTTPS and the setup code required (already enforced), `no-store`, never written to the database, never logged, scrubbed from any barkd error text before it is logged or shown. Input field with autocomplete, autocorrect, autocapitalise and spellcheck off.
3. **Never overwrite.** Refuse if a wallet already exists. Never send `force`. The network must match the server's mode.
4. **After restoring.** Trigger a wallet sync, show the balance, and say plainly that coins can take a round or two to appear. Skip the "write the words down" step but keep a tick-box that the paper copies still exist.
5. **Backup file on the new server.** The upload path already exists for "Check a backup" in Settings. Add the matching wizard choice "I also have a backup file": upload, type its password, restore the database and encryption key, and leave the wallet folder in the file unused. If that proves larger than a small change, fall back to documented DigitalOcean Console steps using the existing `ebp backup restore` command, with an explicit "do not copy the `barkd` folder" step. Sonnet reports which before building.
6. **Tests.** Unit tests for: restore refused when a wallet exists; words never appear in logs or stored settings; wrong network refused; barkd refusal shown as a plain message. Extend the browser e2e with a restore run against the mock wallet. Extend the upgrade/backup CI job to restore a backup onto a clean data folder.

### Order of steps the docs will give (new server)

1. Install a fresh server from `START_HERE.md`, same mode as before.
2. In the setup wizard choose **Restore a wallet I already have** and type the 12 words. Only you present; nothing typed anywhere else.
3. Choose **I also have a backup file**, pick the newest `.ebpbackup` and type the admin password from when it was downloaded.
4. Sign in with that password. Turn two-step sign-in back on.
5. Check Home: the balance, the payout address and the order list.
6. If the server's web address changed, on the WooCommerce site update the server address in the plugin settings. The API key and signing secret are unchanged because they came back with the backup.
7. Download a fresh backup.

### Limits the docs must state plainly

- Orders paid after the last backup download are missing from the payment server's list, but WooCommerce still has them. Download backups regularly.
- A payment that arrived for an open order during the outage may not be matched automatically. The owner compares the shop's On-hold orders with the wallet.
- The words need Second's Ark server to cooperate. If the Ark server is gone, recovery of Ark coins is the emergency-exit path, which needs the wallet folder and is not covered here.
- Without the backup file's password the file cannot be opened. Without the words the money cannot be recovered from the file alone unless the wallet folder in it is still current.

### Signet restore drill (owner runs, before mainnet)

1. On the signet droplet note the balance, order count and payout address. Leave a few hundred sats on the server. Download a backup.
2. Destroy the droplet (or make a second one and leave the first off).
3. Follow the documented steps above exactly as written.
4. Pass: balance, orders, settings and payout address match; a new WooCommerce test order completes; "move my money now" works.
5. Then replace the "exact command is not written down here yet" passages in `docs/RECIPE_BACKUP.md` with the tested steps, update `docs/IF_SOMETHING_LOOKS_WRONG.md` and `docs/KNOWN_ISSUES.md`, and tick the two restore items in `docs/dev/RC_CHECKLIST.md`.

## Stage 2: mainnet code changes (small PRs, in this order)

### PR A: wizard may create or restore a mainnet wallet in live mode
- `src/routes/setup.rs:680-693`: remove the "live mode is disabled" refusal. Allowed network is `"mainnet"` when `mode == "live"`, `"signet"` otherwise. Keep refusing any mismatch in both directions.
- `src/routes/setup.rs:113` and `:695-702`: add `DEFAULT_MAINNET_ASP = "https://ark.second.tech"` and use it when live mode gets no `asp_url`.
- Leave `chain_source` unset (barkd's mainnet default). Only if wallet creation fails in Stage 4, send `{"esplora": {"url": "https://mempool.second.tech/api"}}`.
- `assets/setup.html:145-147` and `:404-409`: in live mode prefill `https://ark.second.tech`.
- Tests near `setup.rs:1669`: live accepts mainnet with the default server; live refuses signet; test mode refuses mainnet.

### PR B: let an install choose live mode
- `docker-compose.yml`: add `EBP_MODE: ${EBP_MODE:-test}` and pass `EBP_MAX_INVOICE_SATS` through (compose passes neither today; `EBP_MODE` is read in `src/config.rs:318`).
- Check `scripts/start.sh`, `scripts/vps-setup.sh` and `deploy/cloud-init.yaml` still work unchanged with the default.

### PR C: addresses on mainnet (owner concern 1)

What already works, to verify and not rewrite:

| Place | What it does today | File |
|---|---|---|
| Payout address check, at setup, in Settings, from the config file at start-up, and again at the moment of every sweep | live mode accepts `ark1…` (bech32m) and `bc1…` (bech32/bech32m with correct program length: `bc1q…` 42 or 62 characters, `bc1p…`); refuses `tark1…`, `tb1…`, `bcrt1…` with "that is a test-network address"; refuses silent-payment `sp1…`; checks the checksum so a typo is caught | `src/payout.rs:149`, `src/config.rs:575`, `src/wallet/settlement.rs:384` |
| Ark or on-chain routing of a sweep | by prefix `ark1` / `tark1` | `src/config.rs:848`, `src/wallet/barkd.rs:350` |
| Addresses shown to a customer at checkout | come straight from barkd; EBP does not check their prefix, so `ark1…`, `bc1…` and `lnbc…` pass through | `src/uri.rs`, `assets/checkout.html:130` |

What to change:
- **Wording that mentions play-money prefixes in live mode:** `assets/setup.html:321-322` and `:671`, `assets/dashboard.html:686`. In live mode say only `ark1…` and `bc1…`; in test mode only `tark1…` and `tb1…`.
- **Accept legacy mainnet addresses (`1…` and `3…`) as payout addresses** (owner decision, 11 November). They are refused today with "that doesn't look like an Ark or Bitcoin address", and many exchanges still hand them out.
  - Scope: live mode only, payout address only. Test-network legacy forms (`m…`, `n…`, `2…`) stay refused.
  - How, with no new dependency: a small Base58Check decoder in `src/payout.rs` next to the existing bech32 one, using the `sha2` crate that is already a dependency. Accept only a 25-byte result whose double-SHA-256 checksum matches and whose version byte is `0x00` (`1…`) or `0x05` (`3…`). Legacy addresses are case-sensitive: do not lowercase them, and skip the "all one case" rule for them.
  - Everything downstream already treats a non-Ark address as on-chain (`src/config.rs:848`), so routing, the on-chain threshold and the fee limit apply unchanged.
  - Tests: known-good `1…` and `3…` vectors accepted in live mode; one changed character refused; both refused in test mode with the "real-Bitcoin address on a test server" message; `m…`/`n…`/`2…` refused.
  - **Check before merging:** barkd 0.8.0's offboard call must accept a legacy destination. Confirm from the pinned OpenAPI and, if it says nothing, on signet with a testnet-style legacy address through `bark` directly. If barkd refuses legacy destinations, do not ship this; keep refusing with the clear message "use an address that starts with bc1 or ark1" and report.
  - Stage 4 sends one real on-chain sweep to a legacy address only if the owner wants to pay that on-chain fee; otherwise the check there is "accepted when saved".
- **Tests.** Add live-mode cases with real-shaped mainnet values: a real `ark1…` address from the owner's phone wallet (not secret), `bc1q…` (42 and 62 characters), `bc1p…`, and a mainnet payment link built from `bc1…`, `lnbc…` and `ark1…` in `src/uri.rs`. The existing `ARK_A` test constant was made up for its checksum only.
- **Open item, still open:** an Ark payout address from a different Ark server passes the check but every sweep to it fails. Refusing it needs the server key inside the address compared with the wallet's server; do that only once the mainnet address format is confirmed in Stage 4. Until then the docs say the payout wallet must be on `ark.second.tech`.

### PR D: coin renewal on mainnet (owner concern 3)

How renewal works today (`src/wallet/settlement.rs:42-68` and `:175-330`):
- Every settlement cycle (5 minutes) it asks the Ark server for its terms (coin lifetime and fee table).
- A coin is renewed when it has fewer blocks left than the smallest of: the configured lead time (7 days), one block inside the server's free window, and a third of the coin lifetime. Never fewer than 12 blocks.
- With mainnet's published terms (28-day coins, free in the last 2 days) that gives 287 blocks, about 48 hours before expiry, inside the free window.
- A failed renewal is recorded and retried on the next cycle; Home and the phone alert warn after repeated failures (`src/alerts.rs:49`).

What the pinned barkd 0.8.0 API says (`docs/barkd-openapi.json`, `/wallet/refresh/vtxos` and `/wallet/rounds`):
- A refresh request **registers** the coins for the next round and answers at once. It does not wait for the round, so EBP's 30-second request limit is not a problem and slow mainnet rounds will not cause false failures. (An earlier version of this plan raised that worry; the API description settles it.)
- The coins are locked straight away, so EBP will not ask again for the same coins while the round is pending. If the round fails, barkd drops it, the coins become spendable again, and EBP's next cycle re-registers them.
- A round counts as finished after 2 confirmations on mainnet (1 on test networks).
- **The coins being refreshed must add up to at least the dust limit (330 sats), and still be at least that after any fee.**

**Change 1 (must have): do not try to renew less than the minimum.**
With a 1,000-sat threshold the server will often hold a small remainder. If the coins due for renewal add up to less than 330 sats, the wallet refuses the request, EBP records a failure every 5 minutes, and Home and the phone alert warn that renewals keep failing, for an amount that cannot be renewed at all. Fix in the renewal step: when the due coins total less than the minimum, add the server's other spendable coins to the same request if that brings the total over the minimum (they are merged into one fresh coin); if the whole balance is still under the minimum, skip quietly, record nothing as failed, raise no alert, and show one plain line on Home ("x sats on the server are too small to renew and will lapse on <date> unless more payments arrive"). Money at stake is under 330 sats. Confirm the exact minimum and whether merging happens from the signet test in Stage 1 before coding.

**Change 2 (should have): say "requested" until the round is done.**
EBP records a renewal as done the moment barkd accepts the registration. On mainnet the round may be hours later. Record it as requested, and mark it done when the coin's expiry has actually moved (the coin list already shows this on the next cycles). This changes what Home and the transfers list say, not when renewals happen. It also makes the mainnet observations in Stage 4 trustworthy.

**Change 3 (small, should have): check the round timing leaves room.**
Read `round_interval` from the server terms (already in the answer; signet reports `5m`). If fewer than 4 rounds fit between the renewal point and expiry, renew earlier even though it costs the fee: not expiring matters more than the fee. With a 2-day free window this only bites if rounds are more than 12 hours apart, so it is a safety net, with a unit test table for 5 minutes, 1 hour, 6 hours and 24 hours.

**No change:** the 7-day fallback when terms cannot be read, the 12-block floor, the "wallet says it needs refresh" top-up.

**Still observed on mainnet with small funds (Stage 4 steps 5, 11 and 14):** the real round interval and how long a registered renewal takes to complete; that short-lived Lightning coins (1 to 3 days) are renewed once and free; that the 28-day coin is renewed inside the free window near day 26; the real fee table parsed as expected, then added as a verified test next to the signet one (`src/wallet/barkd.rs:2717`). Any surprise there becomes a separate owner-approved fix.

### PR E: wording that is wrong on mainnet
- `src/lib.rs:7` ("never mainnet"), `docs/dev/TECHNICAL.md:251`.
- `integrations/woocommerce/ebp-bitcoin/includes/class-ebp-gateway.php:45`: "Test version: use play-money (signet) coins only" is fixed text and the plugin cannot see the server's mode. Reword so it is true in both modes.
- The test banner must stay hidden in live mode (`assets/setup.html:403`, `assets/checkout.html:39`, and the dashboard). In live mode show one line in the same spot: "REAL MONEY. Experimental: keep amounts small."

### Explicitly not changing
The sweep rule (beyond the 1,000 default), the watcher, invoice states, webhook code, the barkd version, the on-chain rail (stays off by default), anything Shopify, and the signet CI workflows.

## Docs (owner concern 2)

**Rule:** a code PR is not finished until every doc that describes the changed behaviour is updated in the same PR, with a `CHANGELOG.md` line.

**Then one final docs sweep PR** before the release. Search every file below for `signet`, `tark1`, `tb1`, `play money`, `play-money`, `test mode`, `faucet`, `20,000` and `not supported`, and make each hit correct for both modes.

| Doc | What changes |
|---|---|
| `README.md` | mainnet is possible but experimental; threshold 1,000 |
| `docs/START_HERE.md` | merge the prepared real-money wording from branch `start-here-live-wording` (rebase first), with "experimental, small amounts"; how to choose live mode; `ark1…` / `bc1…` addresses |
| `docs/WOOCOMMERCE.md`, plugin `readme.txt` | wording true for both modes; test with a product under one dollar on mainnet |
| `docs/RECIPE_BACKUP.md` | the tested restore steps from Part B |
| `docs/IF_SOMETHING_LOOKS_WRONG.md` | lost server; renewal warnings on mainnet; where to ask Second for help |
| `docs/KNOWN_ISSUES.md` | no continuous wallet backup; payout wallet must be on the same Ark server; mainnet is experimental |
| `docs/UPGRADING.md`, `docs/OTHER_HOSTS.md`, `docs/SILENT_PAYMENTS.md`, `docs/KEEPING_WORDPRESS_SAFE.md`, `SECURITY.md` | sweep for the search terms |
| `docs/dev/TECHNICAL.md`, `docs/dev/HANDOFF.md`, `docs/dev/spec.md`, `docs/dev/E2E_TESTING.md` | settings table (`EBP_MODE`, threshold), restore flow, renewal rule |
| `docs/dev/RC_CHECKLIST.md` | section 6: tick what is closed, keep what is open; also see branch `rc-checklist-live-wording` |
| `docs/dev/DROPLET_TEST_SHEET.md`, `docs/dev/SIGNET_DRILL.md` | restore drill; 1,000 threshold |
| new `docs/dev/MAINNET_TEST_SHEET.md` | Stage 4, in the owner's test-sheet style: every step says whether it is on the payment server, the WooCommerce site or the phone wallet |

Dated records (`REVIEW_*`, `DROPLET_TEST_RESULTS_*`, phase and beta plans) are left as they are.

## Stage 3: release

1. After Part B and Stage 2 are merged and CI is green on `main`, follow `docs/dev/RC_CHECKLIST.md` for the next tag: version in `Cargo.toml`, `Cargo.lock`, plugin header and `readme.txt`; zip links in the docs point to the new release page.
2. The owner publishes the pre-release in GitHub's web UI. Verify the Assets list.
3. Re-run the signet "Money safety" section and the restore drill on the new build. Mainnet starts only if both pass.

## Stage 4: mainnet test with small real money (owner runs, Sonnet supports)

**Money limit for the whole stage: about 10,000 sats in total, never more than about 2,000 sats per order.** Set `EBP_MAX_INVOICE_SATS=5000`.

1. **New server.** Fresh droplet from the new release, `.env` with `EBP_MODE=live` and `EBP_MAX_INVOICE_SATS=5000`. Home shows the real-money line and no test banner.
2. **Phone wallet first.** A mainnet wallet in the owner's own Ark wallet on `https://ark.second.tech`; copy its `ark1…` address.
3. **Setup wizard.** Create the wallet (12 words), confirm the three words, admin password, 2FA. Save the `ark1…` payout address.
4. **Address checks on the payment server.** Each of these must be refused with a clear message: a `tark1…` address, a `tb1…` address, an `ark1…` address with one character changed, a `1…` address with one character changed. Each of these must be accepted in Settings (then change back): `bc1q…`, `bc1p…`, `1…`, `3…`.
5. **Capture the server's real terms** with `scripts/drill_capture.sh` before any payment: fee table, coin lifetime, round interval, `ln_receive_anti_dos_required`, minimums. Confirm Home's renewal point is about 2 days before expiry.
6. **First payment, Lightning, about 500 sats, into the empty wallet.** If no Lightning invoice can be made, record the error (the anti-DoS question), pay the first order over Ark, then retry Lightning.
7. **Checkout addresses.** On the checkout page the Ark address starts `ark1`, the Lightning invoice starts `lnbc`, and (if the on-chain rail is on) the Bitcoin address starts `bc1`. The combined QR code is accepted by the phone wallet.
8. **Sweep at 1,000.** Pay about 2,000 sats. Within one settlement interval plus one mainnet round, about 1,000 arrives in the phone wallet and about 1,000 stays. Compare amounts exactly and note any fee and how long the round took.
9. **Overpay by a few sats, underpay once, let one invoice expire.** Same results as on signet.
10. **WooCommerce.** New plugin zip on the test shop, pointed at the mainnet server; buy a product priced under one dollar; order goes to Processing. Then the lost-notice check.
11. **Renewal on a real mainnet round.** The first Lightning payment leaves a coin with 1 to 3 days of life on the server. Watch it get renewed (Home, transfers list) and record how long the request waited for a round and whether any false failure was recorded.
12. **Weekly wallet check card** shows the right figures and folds away after "I checked: it matches".
13. **Backup and restore on mainnet.** Download a backup. Then repeat the restore drill on a second fresh mainnet server with the words and the file, with only the kept 1,000 sats at stake.
14. **Leave it running** with the kept 1,000 sats. Short check after 3 days (no alerts). Long check near day 26 to 28: the 28-day coin is renewed inside the free window, no fee, no expiry warning.
15. **Manual "move my money now"** at the end; server balance is zero.

**Stop conditions (pause payments, do not retry, report):** any sweep or renewal that fails twice; any amount that does not match between server and phone wallet; any alert about coins near expiry; any order marked Paid without money arriving.

## Stage 5: record what mainnet showed

Update `docs/dev/RC_CHECKLIST.md` section 6 with measured answers: Lightning receive fee and what the history reports; the fee table and lifetime as parsed; the anti-DoS setting; the dust limit; the round interval and how long a sweep and a renewal really take; the mainnet Ark address format (for the "different Ark server" check); an emergency-exit cost estimate (read-only, do not start an exit). Anything needing new code becomes a separate, owner-approved item.

## Sources

- https://second.tech/docs/getting-started/bark-cli/mainnet
- https://second.tech/docs/barkd
- https://second.tech/docs/getting-started/bark-cli/signet
- https://second.tech/docs/connection-details.md
- https://second.tech/docs/barkd/start.md and https://second.tech/docs/barkd/install.md
- https://second.tech/docs/learn/lifetime.md and https://second.tech/docs/learn/fees
- https://second.tech/docs/backups.md
- https://second.tech/pricing
