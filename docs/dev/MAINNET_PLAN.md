# Mainnet migration and wallet-restore plan

Revised 11 November 2026 (fifth revision: mainnet terms measured) against `main` at `d3197c3` (`v0.1.0-rc1`). This is a plan, agreed with the owner on 11 November 2026. Nothing in it is built yet except the sweep threshold of 1,000 sats. Whoever does the work follows it in order and stops to report when reality differs from it.

**Goal:** the same features that work on signet today, working on Bitcoin mainnet with very small amounts of real money, plus a documented and tested way to get everything back after losing the server. No other new features and no new dependencies.

## Owner decisions already made

- **Sweep threshold is 1,000 sats** for an Ark payout address, on signet and mainnet (was 20,000). Not near zero: that would create many tiny coins. PR #151.
- **Minimum excess of 1,000 sats.** An automatic sweep sends only once at least 1,000 sats are above the amount kept, so the server holds up to about 2,000 and the payout wallet gets fewer, bigger coins. Chosen after the owner's signet test on 11 November showed sends of 136 and 1,195 sats were accepted but would leave many tiny coins. "Move my money now" ignores the minimum. `MIN_SWEEP_EXCESS_SATS` in `src/wallet/settlement.rs`.
- **Restore from the 12 words will be built** before any real money (Part B). Sonnet does all coding.
- **barkd stays on 0.8.0** during alpha. Updating to the latest Bark happens when moving to beta, as its own piece of work. The 0.8.1 changelog (reviewed 12 November, see **Bark 0.8.1 changelog review** below) does not change this: none of its client fixes touch what EBP uses on mainnet alpha, and one of its changes (`/wallet/send` now pays a bare bitcoin address on-chain) needs a code change before EBP can safely move to it.
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
| Emergency (unilateral) exit | several on-chain transactions per coin; fees paid from the wallet's confirmed on-chain balance; final claim after the exit delta; must start before the coin expires | barkd OpenAPI `/exits/*`, learn/lifetime |
| Coin lifetime | 28 days for boards and refreshes; 1 to 3 days for coins from Lightning receives | learn/lifetime |
| Refresh fee | free in the last 2 days before expiry, then 0.2% / 0.4% / 0.5% | second.tech/pricing, learn/fees |
| Lightning receive, Ark send, Ark receive | 0% | second.tech/pricing |
| Rounds | **every 1 hour on mainnet** (signet: 5 minutes); a refresh waits for the next round | measured 11 Nov (below), bark-cli/mainnet |
| Second's advice for mainnet | start with small amounts; save the seed phrase; back up the wallet data folder continuously; only store what you can afford to lose | bark-cli/mainnet |
| Mainnet support | chat.second.tech, contact Neil (982572) | bark-cli/mainnet |

**Measured on 11 November 2026** by running the project's `mainnet-compat-check` workflow (the exact checksum-pinned barkd 0.8.0 build, a throwaway empty mainnet wallet, no money; run 43126312755):

| Measured | Value |
|---|---|
| barkd 0.8.0 against `ark.second.tech` | **connects and creates a mainnet wallet** (`connected: true`). No upgrade to 0.8.1 is needed for this. |
| Round interval | `1h` |
| Coin lifetime / exit delta | 4032 blocks (28 days) / 144 blocks (1 day) |
| Refresh fee | 0% when under 288 blocks (2 days) to expiry; 0.2% from 288; 0.4% from 1008; 0.5% from 2016 |
| `ln_receive_anti_dos_required` | `false`: Lightning receive should work into an empty wallet |
| Lightning receive fee | 0 |
| Largest single coin | 10,000,000 sats |
| Smallest on-chain board | 50,000 sats (EBP does not board; the on-chain rail stays off) |
| On-chain payout (offboard) fee | 0.2% to 0.5% of the amount, plus about 389 vbytes at the current feerate (124 sat/kvb at the time, about 48 sats) |
| Server terms of service | https://second.tech/terms |

Still not known: the smallest Ark send (dust limit, expected 330 sats, to be confirmed in Stage 1 and 4).

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

How the threshold behaves: the server keeps 1,000 sats and sends on what is above that, but only once at least 1,000 sats are above it (`MIN_SWEEP_EXCESS_SATS`). Result of the owner's test on 11 November, before the minimum was added: 19,000, 1,195 and 136 sats were all sent on and accepted by Ark (the 136-sat renewal attempted just before it failed, as expected for a coin that small), which is why the minimum was added.

1. Done 11 November: a 20,000 balance went down to 1,000 within minutes of the update (19,000 sent).
2. Done 11 November: 1,195- and 136-sat sends were accepted by Ark on signet. So Ark takes out-of-round sends below the believed 330-sat limit, at least on signet; confirm on mainnet in Stage 4.
3. Still to watch: the 136-sat coin's renewal failed, and failed renewals feed the "renewing is not working" warning (see the renewal section).
4. Renewal while waiting for a round (for PR D Change 4): the next time a "Routine wallet upkeep" row appears, run `./scripts/drill_capture.sh` in `/opt/ebp` within a minute and again 5 minutes later, and keep both files.

**Retest after the minimum is deployed:** with 1,000 held, pay 300 sats (nothing sent), then 800 sats (about 1,100 sent in one transfer).

## Bark 0.8.1 changelog review (12 November 2026)

Source: https://second.tech/docs/changelog (0.8.1 released 5 November 2026; nothing newer listed). Checked against the barkd calls EBP makes (`src/wallet/barkd.rs` paths, `src/managed_barkd.rs` start-up).

**Matters now, while we stay on 0.8.0** (these are Ark-server changes, or 0.8.0 behaviour we had not accounted for):
- **Round registration does not lock coins (0.8.0), and Second's server now refuses a participation that would replace one a round attempt holds (0.8.1 server).** This corrects PR D and adds Change 4 (never ask twice). Must be done before mainnet.
- **barkd serves a wallet web page on its own port (0.8.0, `--no-ui` turns it off).** EBP starts barkd on `127.0.0.1` inside the container, so it is not reachable from outside today. Still, add `--no-ui` to the barkd start command in `src/managed_barkd.rs` and to the `mainnet-compat-check` workflow (which proves the pinned binary accepts the flag before it ships). Small hardening, its own PR (call it PR F), before mainnet.
- **Restore from the words asks the Ark server for coins by key first (0.8.0), about 60 seconds instead of 210 in Second's measurement, and recovers round outputs that finished while the wallet was offline.** Good for Part B; the wizard allows 5 minutes. The owner's drill records how long it really takes.
- **Second's mainnet server now refuses a zero `vtxo_exit_delta`.** No effect on us; mainnet measured 144 blocks.

**Matters when moving to 0.8.1 (beta), not before:**
- **`POST /wallet/send` now pays a bare bitcoin address on-chain** (before, it refused), and picks Ark, then Lightning, then on-chain when no `method` is given. EBP's Ark sweep calls `/wallet/send` (`src/wallet/barkd.rs` `PATH_ARK_SEND`) with only `destination` and `amount_sat`. Before upgrading, send `"method": "ark"` with every Ark sweep, so a mistake can never turn into an on-chain payment with fees. On 0.8.0 a bare address is refused, which is the safer behaviour for alpha.
- **`GET /lightning/receives/{id}` answers 404 (not 500) for an unknown or cancelled receive.** EBP does not call it today; check again at upgrade time.
- **Lightning `state` fields become enums in the OpenAPI** (same JSON values). The `barkd openapi drift check` in CI will flag the schema change; refresh `docs/barkd-openapi.json` as part of the upgrade.
- **`GET /wallet/debug-info`** (network, mailbox ID, VTXO xpub): useful to prove a restored wallet is the same wallet as before (same xpub). Add to `scripts/drill_capture.sh` at upgrade time.
- **Presigned emergency exits** (`POST /api/v1/exits/presign`, GitLab MR !2293; `GET /api/v1/exits/remaining`): signs the fee-bumping transactions in advance, paid from confirmed on-chain funds, into a plan anyone can broadcast later without the wallet keys. Could let an owner keep an exit plan off the server. Limits: it covers only the coins that exist when it is made (EBP's coins change with every payment, sweep and renewal, so it goes stale quickly), and it still needs on-chain funds set aside. Consider at beta on top of PR G; not needed for PR G itself, which uses the 0.8.0 exit calls.
- Lightning invoice custom expiry: EBP already sends `expiry_secs` (pinned 0.8.0 API has it). 0.8.1 makes barkd fail the request if a server ignores it; Second's server supports it, so no change expected.

**No effect on EBP:** Lightning *send* fixes (EBP never sends over Lightning), emergency-exit presigning and `exits/remaining` (exits stay manual), Bitcoin Core v32 mempool fix (EBP uses Esplora), the new `bark-cli`, BDK and Rust SDK breaking changes (EBP talks REST only), WebSocket tickets, WASM, gRPC compression, server operator items (Core Lightning version, `watchmand`, backfill commands).

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

### Build (done: PR #159, merged 12 November; owner drill still to do)

Built as described below, with the backup file restored by the wizard itself: the server stages the backup's database and key, restarts, and swaps them in before anything is opened, keeping the files they replace in `restore-previous-<time>/`. Steps for owners: `docs/RECIPE_BACKUP.md`.


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

**Passed 12 November 2026** on a second droplet ([RESTORE_DRILL.md](../RESTORE_DRILL.md)): balance (1,000 sats), orders, transfers list, security log, payout address and shop connection all came back; a new 1,279-sat test order completed and "Move my money now" delivered it to the owner's Bark wallet; two-step sign-in turned back on; a fresh backup from the new server passed **Check a backup**. Restore took 30 to 40 minutes, 17 of them the new server's first start. One extra "Routine wallet upkeep" row appeared right after the restore (1,000 sats, broadcast; balance unchanged): the new server renewed its coin on its first cycle. Not explained yet; watch for it in the mainnet restore drill (Stage 4 step 13).

1. On the signet droplet note the balance, order count and payout address. Leave a few hundred sats on the server. Download a backup.
2. Power off the droplet (or make a second one and keep the first powered off). Never run both at once: same wallet words, same shop connection.
3. Follow the documented steps above exactly as written.
4. Pass: balance, orders, settings and payout address match; a new WooCommerce test order completes; "move my money now" works.
5. Then replace the "exact command is not written down here yet" passages in `docs/RECIPE_BACKUP.md` with the tested steps, update `docs/IF_SOMETHING_LOOKS_WRONG.md` and `docs/KNOWN_ISSUES.md`, and tick the two restore items in `docs/dev/RC_CHECKLIST.md`.

## Stage 2: mainnet code changes (small PRs, in this order)

**Live mainnet test, 13 November 2026** (`live-test-mainnet` branch, workflow run 43442570273, owner's 5,000 sats): two real barkd 0.8.0 wallets on `ark.second.tech` driven by EBP's own code. Lightning receive 5,000 sats (no fee); Ark send 2,500 A to B (no fee); Lightning B to A 400 sats (20-sat fee); EBP's settlement engine in live mode kept exactly 1,000 and sent 1,900 to B's Ark address on its first cycle; "Move my money now" returned 1,000 + 3,980 to the owner (4,980 of 5,000; the difference is the Lightning fee). Unfunded checks: network `bitcoin`, round_interval 1 h, coin lifetime 4,032, free window 288, exit delta 144, real `ark1…` and `bc1p…` addresses pass the live checks. **Emergency withdrawal estimate for one 1,000-sat coin: 6,444 sats (15 transactions) at 1.08 sat/vB**, so an exit costs thousands of sats per coin even when fees are low; the 100,000-sat and twice-the-fees rule refused it. Not covered: renewal (the coins were too new).

**Status 13 November 2026:** all seven are built as a stack of draft PRs, not merged (owner cuts `v0.1.0-rc2` first): A #171, B #172, C #173, D #174, E #175, F #176, G #177. Merge in that order after #160. Where the build differs from this plan, the PR says so; the main points: B adds a red Home line when the wallet's network does not match the mode; D's `/wallet/rounds` parsing follows the pinned OpenAPI because the Stage 1 capture was not taken; G stays switched off on real-money servers until its signet drill; F was verified with the compat workflow (run 43372649041). Mainnet's ark-info says `"network": "bitcoin"` (captured by the compat run), now a test fixture.

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
- **Done (14 Nov):** an Ark address carries the first 4 bytes of SHA-256 of its server's public key (confirmed on five mainnet addresses and three signet ones). The setup page compares that with `server_pubkey` from ark-info (verified / could not verify / failed, continuing allowed after a danger warning), and sweeps refuse a definite mismatch, with a message naming `ark.second.tech` and Noah (Android) / Arké (iPhone).

### PR D: coin renewal on mainnet (owner concern 3)

How renewal works today (`src/wallet/settlement.rs:42-68` and `:175-330`):
- Every settlement cycle (5 minutes) it asks the Ark server for its terms (coin lifetime and fee table).
- A coin is renewed when it has fewer blocks left than the smallest of: the configured lead time (7 days), one block inside the server's free window, and a third of the coin lifetime. Never fewer than 12 blocks.
- With mainnet's published terms (28-day coins, free in the last 2 days) that gives 287 blocks, about 48 hours before expiry, inside the free window.
- A failed renewal is recorded and retried on the next cycle; Home and the phone alert warn after repeated failures (`src/alerts.rs:49`).

What the pinned barkd 0.8.0 API says (`docs/barkd-openapi.json`, `/wallet/refresh/vtxos` and `/wallet/rounds`):
- A refresh request **registers** the coins for the next round and answers at once. It does not wait for the round, so EBP's 30-second request limit is not a problem and slow mainnet rounds will not cause false failures. (An earlier version of this plan raised that worry; the API description settles it.)
- ~~The coins are locked straight away.~~ **Corrected 12 November:** the Bark 0.8.0 changelog says "joining a round no longer locks input VTXOs; locks apply only once a round attempt starts". So between the request and the start of the round the coins may still be listed as spendable. On signet (frequent rounds) the owner saw one renewal row and no repeats, so in practice they dropped out of the spendable list quickly; on mainnet (hourly rounds) that window can be up to an hour. See Change 4.
- A round counts as finished after 2 confirmations on mainnet (1 on test networks).
- **The coins being refreshed must add up to at least the dust limit (330 sats), and still be at least that after any fee.**

**Change 1 (must have): do not try to renew less than the minimum.**
With a 1,000-sat threshold the server will often hold a small remainder. If the coins due for renewal add up to less than 330 sats, the wallet refuses the request, EBP records a failure every 5 minutes, and Home and the phone alert warn that renewals keep failing, for an amount that cannot be renewed at all. Fix in the renewal step: when the due coins total less than the minimum, add the server's other spendable coins to the same request if that brings the total over the minimum (they are merged into one fresh coin); if the whole balance is still under the minimum, skip quietly, record nothing as failed, raise no alert, and show one plain line on Home ("x sats on the server are too small to renew and will lapse on <date> unless more payments arrive"). Money at stake is under 330 sats. Confirm the exact minimum and whether merging happens from the signet test in Stage 1 before coding.

**Change 2 (should have): say "requested" until the round is done.**
EBP records a renewal as done the moment barkd accepts the registration. On mainnet the round may be hours later. Record it as requested, and mark it done when the coin's expiry has actually moved (the coin list already shows this on the next cycles). This changes what Home and the transfers list say, not when renewals happen. It also makes the mainnet observations in Stage 4 trustworthy.

**Change 3 (small, should have): check the round timing leaves room.**
Read `round_interval` from the server terms (the answer includes it as text such as `1h`; EBP does not read it today). If fewer than 4 rounds fit between the renewal point and expiry, renew earlier even though it costs the fee: not expiring matters more than the fee. Mainnet measured `1h` rounds, so the 2-day free window holds about 48 rounds and this will not trigger today. It is a safety net in case Second changes the interval, with a unit test table for 5 minutes, 1 hour, 6 hours and 24 hours.

**Change 4 (must have, added after the 0.8.1 changelog review): never ask twice for coins already waiting for a round.**
EBP's renewal step picks due coins from the spendable list every 5 minutes. If coins already registered for the next round are still listed as spendable (see the correction above), EBP would register them again every cycle for up to an hour on mainnet. Each extra request adds a "Routine wallet upkeep" row, holds up the sweep (the sweep waits for the cycle after an accepted renewal), and, since Second's 0.8.1 server "refuses a delegated round participation that would replace one a round attempt already holds" and names those inputs, can come back as an error that EBP counts as a failed renewal, which after a few cycles shows the red "renewing is not working" warning and sends a phone alert for nothing. Fix in the renewal step:
- Before asking, read `GET /api/v1/wallet/rounds` (already in the pinned 0.8.0 API, read-only) and leave out any coin that is an input of a round participation still in progress.
- If barkd still refuses because a round attempt holds the inputs, record it as "already being renewed" (not a failure; no alert), and try again next cycle.
- Unit tests: a pending participation holding a coin means no new request for that coin; the "inputs held" refusal does not count towards the failure warning.
- **Measure first (Stage 1):** right after a renewal row appears on Home, run the rounds command (in `/opt/ebp`) once, then again 5 minutes later, and save both outputs: they show whether the coins are still listed as spendable while they wait, and the exact shape of `/wallet/rounds` to code against. `scripts/drill_capture.sh` already captures both.

**No change:** the 7-day fallback when terms cannot be read, the 12-block floor, the "wallet says it needs refresh" top-up.

**Still observed on mainnet with small funds (Stage 4 steps 5, 11 and 14):** how long a registered renewal takes to complete (rounds are hourly); that short-lived Lightning coins (1 to 3 days) are renewed once and free; that the 28-day coin is renewed inside the free window near day 26; the real fee table parsed as expected, then added as a verified test next to the signet one (`src/wallet/barkd.rs:2717`). Any surprise there becomes a separate owner-approved fix.

### PR E: wording that is wrong on mainnet
- `src/lib.rs:7` ("never mainnet"), `docs/dev/TECHNICAL.md:251`.
- `integrations/woocommerce/ebp-bitcoin/includes/class-ebp-gateway.php:45`: "Test version: use play-money (signet) coins only" is fixed text and the plugin cannot see the server's mode. Reword so it is true in both modes.
- The test banner must stay hidden in live mode (`assets/setup.html:403`, `assets/checkout.html:39`, and the dashboard). In live mode show one line in the same spot: "REAL MONEY. Experimental: keep amounts small."

### PR G: emergency withdrawal without the Ark server (owner request, 12 November)

**Why.** Today EBP has no way to get coins out if Second's Ark server goes offline or refuses to cooperate. The code says "emergency exits are never automated — they are a documented runbook", but no runbook exists. Ark's promise is that the owner always holds the keys and can leave alone (a unilateral exit); EBP should make that possible from the dashboard, as a last resort.

**What barkd 0.8.0 already offers** (pinned `docs/barkd-openapi.json`, no new dependency): `GET /api/v1/exits/fee` (estimate, read-only), `POST /api/v1/exits/start/all` (register every coin for exit; barkd then broadcasts the exit transactions by itself, in order, over time), `GET /api/v1/exits/status/live` and `/finished`, `POST /api/v1/exits/cancel/{vtxo_id}` (allowed until the last transaction is broadcast), `POST /api/v1/exits/claim/all` (manual last step: one on-chain transaction to an address). The fees for each step are paid from the server's **confirmed on-chain balance** (fee bumping), not from the coins being exited.

**Facts the owner must know before using it** (and the page must say):
- It is several on-chain transactions per coin, each paying network fees, then a wait of at least the exit delta (144 blocks, about a day, on mainnet) before the final claim. Expect a day or more, and fees that can be large when the network is busy.
- **With the 1,000-sat keep amount the server holds very little, so an exit would usually cost more in fees than it rescues.** The money that matters is in the owner's own payout wallet, whose own app is responsible for its exits. So the server refuses an exit that is not worth it (step 3a below).
- **It must start before the coins expire** (minus the exit delta). Lightning-received coins live only 1 to 3 days, refreshed ones 28 days. After that the Ark server can take them back (barkd reports `vtxo-swept`). So the page shows the deadline.
- The server must stay on for the whole exit. An exit in progress is not restored from the 12 words; the wallet folder (in the backup file) carries it.

**Where and how (one PR, plus a signet drill before it is offered on mainnet):**
1. **Settings → "Emergency: withdraw without the Ark server"**, folded away by default under a red heading. Never on Home as a button.
2. **The warning**, at the top, in plain words: "You always hold the keys to this money, so you can take it out on your own even if the Ark server is gone or refuses to help. This is for emergencies only. It takes several Bitcoin transactions, each paying network fees, and at least a day. Fees can be high, and on small amounts they can be more than the money itself. If the Ark server is working, use **Move my money now** instead: it is cheaper and faster."
3. **Before anything can start, the page shows:** the Ark balance on the server; the fee estimate from `exits/fee` at the current network fee rate; the server's confirmed on-chain balance (which pays those fees); the deadline (earliest coin expiry minus the exit delta); and whether the Ark server answers right now (`wallet/connected`).
3a. **Refuse when it is not worth it (owner rule, 12 November).** The exit can only start when the Ark balance on the server is **at least 100,000 sats AND at least double the projected on-chain fees** (broadcast plus claim, from `GET /exits/fee` at the current rate, including barkd's margin). Both must hold. Otherwise the start button stays disabled and the page says: "Not worth it right now. The network fees to take this money out on your own (about X sats) are too high compared with what is on the server (Y sats). It is more economical to leave it, or to wait and hope the Ark server comes back online and cooperates. Fees change: check again later." If the fee estimate cannot be read, the exit is refused too ("Could not estimate the fees right now; try again later"). The check is made on the server (not only in the page) when the exit is started, with the fee estimate refreshed at that moment. With the default 1,000-sat keep amount this means the emergency withdrawal will normally be refused; it is there for owners who keep more on the server. Tests: 99,999 sats with tiny fees refused; 200,000 sats with fees of 100,001 refused; 200,000 with fees of 100,000 allowed; estimate unavailable refused.
4. **Paying the fees: a guided step, not a dead end.** The fees are paid from the server's own on-chain wallet, which is normally empty, so the page walks the owner through filling it:
   - **How much:** the broadcast cost from `GET /exits/fee` (no `vtxo_ids`, so the whole wallet; barkd's default `fee_margin` of 1.2 already adds 20% for fee rises), shown as "Send at least X sats". The claim cost is shown separately: it comes out of the exited coins at the end, not from this deposit.
   - **Where:** the server's own on-chain address (`POST /onchain/addresses/next`) as text with a Copy button and a QR code, the same way the checkout page shows addresses. Any wallet or exchange can send to it.
   - **Waiting:** the page re-checks every minute (`POST /onchain/sync`, then `GET /onchain/balance`) and shows "Received X of Y sats, waiting for 1 confirmation". Only confirmed coins can pay exit fees, so the start button unlocks only once the **confirmed** balance covers the estimate, and the estimate is refreshed at that moment (fees may have moved).
   - **If fees rise mid-exit** and the deposit runs out, barkd stops bumping; the page shows "Add X sats to keep going" with the same address. The exit carries on once more has confirmed.
   - **Leftover:** when the exit is claimed, any fee money left over is offered back with a **Send leftover to my address** button (`POST /onchain/drain` to the same destination, password and two-step code), so nothing is stranded on the server.
   - Check in the PR that the sweep never touches this on-chain balance (today the sweep only moves Ark coins: `src/wallet/settlement.rs` test `ark_sweep_does_not_need_the_exit_reserve`).
5. **Destination.** An on-chain address the owner controls: the payout address if it is an on-chain one, otherwise typed in and checked with the same rules as the payout address (mainnet `bc1…`, plus `1…`/`3…` if PR C lands). Never an Ark address.
6. **Confirmation.** A tick-box "I understand this is for emergencies, costs network fees and takes at least a day", the admin password, **and** the two-step code. Two-step sign-in must be on; if it is off, the page says to turn it on first and the start button stays disabled. Same server-side checks as changing the payout address (`check_second_step`), audit-logged (`exit.started`).
7. **While it runs.** The settlement engine pauses renewals and sweeps while any exit is live (they would fight over the same coins). Home shows a red line "Emergency withdrawal in progress since <time>" with a link to Settings. Settings lists each coin's exit state from `exits/status/live`. A **Stop** button (password and two-step code again) calls `exits/cancel` for coins where barkd still allows it. The page tells the owner to download a fresh backup now, because the backup carries the exit and the words do not.
8. **Claiming.** When barkd reports the outputs are claimable, a phone alert and a Home line say so, and a **Send to my address** button (password and two-step code) calls `exits/claim/all` with the destination. Claiming is never automatic.
9. **Never automatic, never on a timer.** Nothing starts an exit except the owner pressing the button.
10. **Tests.** Unit and API tests with the mock wallet: refused below 100,000 sats or below twice the fees (step 3a), refused without the password, without the two-step code, with two-step off, with too little on-chain balance, with an Ark destination; settlement pauses while an exit is live; claim and stop need the second step; the warning text is on the page. Browser e2e: the section is folded away and the button stays disabled until the tick-box, password and code are given.
11. **Signet drill (owner, before the option is enabled on mainnet).** With at least 150,000 play-money sats held on the server (raise the keep amount in Settings to 200,000 for the drill so the sweep leaves them there, and put it back afterwards; also try once below 100,000 to see the refusal): fund the on-chain fee balance from a faucet, start an exit, watch it progress, claim to a signet address, and write down how long each step took and what it cost. Capture the `exits/status` shapes with `scripts/drill_capture.sh` (it already reads exits). Mainnet stays read-only: Stage 5 records a fee estimate only, no real exit.

**Order.** After PR D (renewal) and before Stage 3. If it is not ready by then, the release notes and `KNOWN_ISSUES.md` must say plainly that there is no emergency withdrawal yet, and the amounts kept on the server stay small (the 1,000 default) until it ships.

### PR F: turn off barkd's built-in wallet page (added 12 November)
Add `--no-ui` to the barkd start command (`src/managed_barkd.rs`, `spawn`) and to `.github/workflows/mainnet-compat-check.yml`. Run that workflow first: if the pinned 0.8.0 binary refuses the flag, drop this PR and note it (the page is only on `127.0.0.1` inside the container). Test: the managed start command includes the flag.

### Explicitly not changing
The sweep rule (beyond the 1,000 default and the 1,000 minimum excess), the watcher, invoice states, webhook code, the barkd version, the on-chain rail (stays off by default), anything Shopify, and the signet CI workflows.

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
5. **Capture the server's real terms** again with `scripts/drill_capture.sh` before any payment and compare with the 11 November measurements (1-hour rounds, 28-day coins, free renewal under 288 blocks, anti-DoS off). Confirm Home's renewal point is about 2 days before expiry.
6. **First payment, Lightning, about 500 sats, into the empty wallet.** The mainnet server reports `ln_receive_anti_dos_required: false`, so this should work. If no Lightning invoice can be made, record the error, pay the first order over Ark, then retry Lightning.
7. **Checkout addresses.** On the checkout page the Ark address starts `ark1`, the Lightning invoice starts `lnbc`, and (if the on-chain rail is on) the Bitcoin address starts `bc1`. The combined QR code is accepted by the phone wallet.
8. **Sweep at 1,000.** Pay about 2,000 sats. A payment that came in over Lightning lands as a short-lived coin that the server renews first (the owner's signet test on 11 November: renewal at 8:14 PM, sweep about 10 minutes later, with 5-minute rounds); the sweep only happens after that renewal round finishes. With hourly mainnet rounds expect the money to reach the phone wallet after about 5 to 65 minutes (on average about half an hour), not 5. An on-chain payout also waits for a round. Do not treat a wait of under about 75 minutes as a failure on mainnet; record the real time. About 1,000 arrives in the phone wallet and about 1,000 stays. Compare amounts exactly and note any fee and how long it took.
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
- https://second.tech/docs/changelog (0.8.0 and 0.8.1 reviewed 12 November 2026)
