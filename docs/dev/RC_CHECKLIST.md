# Release candidate checklist

Goal of the first tag: a **release candidate** (`v0.1.0-rc1`) that testers can
install with play money (signet) and that has a documented, short list of
things it does not do. **Not** a "ready for real money" release.

Nothing here is published until a person pushes a tag. Tags on a public
repository cannot be taken back cleanly, so the last section lists exactly
what to check first.

## 1. What CI proves on every change to `main`

| Check | What it covers |
| --- | --- |
| build, tests, clippy, fmt | The Rust code, including unit tests |
| Browser end-to-end | Setup wizard, recovery-words quiz, admin login, Ark payment, on-chain payment and replacement, outage and a hung wallet, restart, underpay, test payment, encrypted backup, payout change, every request matches barkd's pinned API spec |
| WooCommerce end-to-end | A real WordPress + WooCommerce: checkout redirect, forged notice refused, partial and full payment, repeat notice ignored, return link, shop-side tripwire, changed order total not auto-paid |
| Upgrade and restore | Previous build upgraded in place keeps data; encrypted backup, verify (wrong password refused) and restore |
| DigitalOcean rehearsal | The paste-in setup run on a clean Ubuntu 24.04 machine, minus the real internet certificate |
| Scripts and blueprints | shellcheck, cloud-init validity, Render blueprint safety rules |
| barkd API drift | Our client matches barkd 0.8.0's published API |
| Dependency audit | Known-vulnerability scan |
| Docker image | Builds for amd64 and arm64 |
| Release dry run (manual) | Builds both binaries, packages the plugin, checksums, smoke-tests the binary; publishes and signs nothing |

## 2. What CI cannot prove (a person must do these)

Record the result of each. A box ticked without a real run is worse than an
open box.

- [ ] **Signet drill** (`SIGNET_DRILL.md`): the real barkd, a real Ark server
      and a phone wallet. Biggest gap. Until it is done the first-run wizard
      with the real wallet daemon, the shape of real payments, and the
      on-chain rail are unconfirmed.
- [ ] **DigitalOcean on a real droplet**: paste `deploy/cloud-init.yaml`, wait,
      read the Console, finish setup in a phone browser. Note exactly what the
      Console showed and how long it took. This is the first test of the real
      internet certificate (sslip.io + Let's Encrypt). First run done on 8 November
      2026 (SFO3, signet): see `DROPLET_TEST_RESULTS_2026-11-08.md`. Keep this open
      until the second round (with WooCommerce) is done.
- [ ] **WooCommerce on a real shop host** (not the CI one), with the plugin zip
      from the dry run.
- [ ] **Phone alerts** with the real ntfy app: scan the QR code, send a test.
- [ ] **Backup restore on a second machine**, from a file downloaded through the
      browser.
- [ ] **A newcomer reads `START_HERE.md`** and gets to a paid test order without
      help. Write down every place they hesitate.
- [ ] Render: only if you want it listed as available. Otherwise leave it
      "planned".

## 3. Decisions needed before the tag

- [ ] **Public name.** The GitHub account name appears in the clone address in
      the docs and `cloud-init.yaml`, in the update-check address in
      `src/updates.rs`, in `scripts/install.sh`, and in the image name. Change it
      **before** the tag (after a rename, old addresses only redirect until
      someone else registers the old name). Commits made so far also carry the
      old name inside the saved author address; hiding that needs a history
      rewrite or a fresh repository, so decide now.
- [ ] **Security reports.** `SECURITY.md` points people to GitHub's private
      reporting. Turn that on in the repository settings (Settings → Code
      security → Private vulnerability reporting), or the link leads nowhere.
      `LICENSE` is present; confirm it is the one you want.
- [ ] **Protect the `v*` tags** in the repository settings (so only you can
      create them).
- [ ] Delete stale branches, especially `mut-test` (never merge it).

## 4. Release steps (in order)

1. `main` is green and the manual **Release dry run** (Actions → release → Run
   workflow, on `main`) is green. Download the `dry-run-release-files` artifact
   and look at it: two `ebp-*.tar.gz`, the plugin zip, `SHA256SUMS`.
   The image job takes well over half an hour (arm64 under emulation).
2. Move the `Unreleased` section of `CHANGELOG.md` under a `0.1.0-rc1` heading,
   and check the version matches the tag in `Cargo.toml`, `Cargo.lock`, and the
   plugin (`ebp-bitcoin.php` header and `EBP_BITCOIN_VERSION`, `readme.txt` Stable tag),
   so Home and the Plugins page show which build is running.
3. Tag `v0.1.0-rc1` on `main` and push it. The `-rc1` suffix makes the workflow
   publish a **pre-release**, which GitHub's "latest" and the dashboard's update
   notice both ignore. Install an rc by explicit version, not `latest`.
4. Watch the release run. Then check: assets present, `SHA256SUMS` matches,
   the signature bundle exists, the notes show the image digest and the verify
   command, and the verify command works on your machine.
5. Install that release on a fresh server exactly as `START_HERE.md` says, using
   the digest-pinned image.
6. Only then tell testers. Final `v0.1.0` comes after the checklist in
   section 2 is fully ticked; the dashboard will tell rc users when it exists.

## 5. Draft release notes (edit after the drills)

For a release candidate with the mainnet changes, replace the signet-only line
below with: "Play money (signet) is the default. Real bitcoin can be switched on
(`EBP_MODE=live`, see START_HERE) but is experimental and not yet tried with real
money: if you do, use only a few thousand sats you can afford to lose. The
emergency withdrawal is switched off on real money for now."

> **v0.1.0-rc1 — release candidate, play money only**
>
> Easy Bitcoin Payments takes Bitcoin payments (over Ark) into a wallet you
> control, with a WooCommerce plugin and a no-terminal DigitalOcean install.
>
> **Please test with signet coins only. Do not use real bitcoin.**
>
> What works (tested automatically): setup wizard, payments (Ark, and on-chain
> when you turn it on), underpay/overpay handling, webhooks, WooCommerce
> checkout and order updates, encrypted backup and restore, upgrades that keep
> your data, phone alerts, a weekly own-wallet check, a shop-side warning if the
> payment server stops answering or orders stop being paid.
>
> Not yet verified by a person on live infrastructure: the real-wallet signet
> run, a real DigitalOcean server, phone alerts on a real phone.
>
> Not included: Shopify, Lightning payouts, refunds, automatic updates.
>
> Install: `docs/START_HERE.md` (no terminal) or `docs/START_HERE.md`.
> Verify the download with the checksums and signature in this release.

## 6. When live mode exists

Decision (8 Nov 2026): the public walkthrough `docs/START_HERE.md` stays in its
play-money wording until live mode is supported. The version written for real use
(no test or signet wording, `ark1` / `bc1` addresses) is ready on branch
`start-here-live-wording`. Merge it in the same release that enables live mode,
rebase it first, and drop its note in `DROPLET_TEST_SHEET.md` if testing is then
done differently.

**The agreed plan for getting to mainnet is [MAINNET_PLAN.md](MAINNET_PLAN.md)** (11 November
2026): restore from the 12 words first, then the mainnet changes, then a test with small
real money. Its items, in order:

- [ ] **Sweep threshold of 1,000 sats tested on signet**, including a 200-sat and a
      500-sat order (one below and one above the wallet's 330-sat minimum).
- [ ] **Restore from the 12 words plus the backup file**, built and drilled on signet
      (the plan's Part B). The words bring back the money; the backup file brings back
      orders, settings and the shop connection.
- [ ] **Wizard can create a mainnet wallet in live mode** (`https://ark.second.tech`), and
      an install can choose live mode (`EBP_MODE=live`).
- [ ] **Mainnet addresses.** Hints and messages name `ark1…` / `bc1…` in live mode; legacy
      `1…` and `3…` payout addresses accepted if barkd can pay to them; tests with
      mainnet-shaped addresses.
- [ ] **Renewal: skip amounts below the 330-sat minimum.** barkd refuses to refresh less
      than the dust limit. Merge with the server's other coins when that gets over the
      minimum; otherwise skip quietly, record no failure, raise no alert, and say so on Home.
- [ ] **Renewal: show "requested" until the round is done.** barkd registers a refresh
      for the next round and answers at once; today that is recorded as done.
- [ ] **Renewal: renew earlier if rounds are very slow** (fewer than 4 rounds between the
      renewal point and expiry), even though it costs the fee.
- [ ] **Docs sweep for both modes**, then the mainnet test sheet with small real money.

barkd stays on 0.8.0 during alpha; moving to the latest Bark is its own piece of work at beta.

### Follow-ups from the 9 November review (need mainnet or a later test)

- [ ] **Lightning receive fee on a server that charges one.** Checkout now asks the
      customer for the order plus the quoted fee and counts a payment of the shop's own
      Lightning invoice as paid in full. Confirm on mainnet (signet's fee is 0) what the
      wallet history reports as received, and that orders show Paid.
- [ ] **Ark payout address on a different Ark server.** Sweeps to it fail forever. Refuse
      it when saved by comparing the server key inside the address with the wallet's own
      server (needs the exact mainnet address format confirmed first).
- [ ] **Renewal fees.** Extra renewals of coins the wallet flags now stay inside the free
      window. Confirm the mainnet fee table and coin lifetime parse as expected, whether
      renewing merges small coins, and show renewal fees paid on Home.
- [x] **Restore from the 12 words (top priority before real money).** The setup page
      now has "Restore the wallet I already have" and a backup-file step (built, covered
      by automated tests). Still to do: drill it once for real on signet (second server),
      then tick this and the next item.
- [x] **Backup restore drill.** Passed 12 November 2026 on signet (see MAINNET_PLAN.md). Follow RECIPE_BACKUP.md "Practice a restore once" on a
      second signet server and fix any step that does not match the page.
- [ ] **Ark payout coins expire in the owner's wallet (28 days on Second's server).**
      Sweeps send existing coins whole (a 9 Nov signet sweep delivered three coins that
      kept their original expiry heights), so the owner's wallet app must renew them (EBP cannot
      see or control it). Tell the owner plainly in the payout step of setup ("check your wallet
      app renews coins by itself, or choose an on-chain address for long-term storage"), and
      check on signet after two weeks that the owner's wallet really renewed them.
- [ ] **Ark payments made with nearly expired coins.** A coin paid over Ark keeps the
      sender's expiry and the order is marked Paid when it arrives. Decide a minimum
      time left (or wait for the renewal on larger amounts) before marking Paid; needs
      a way to tie a received payment to its coin's expiry.
- [ ] **Lightning for a brand-new mainnet shop.** The Ark server setting
      `ln_receive_anti_dos_required` is off on signet. If mainnet turns it on and it
      needs existing coins, an empty wallet cannot make Lightning invoices while
      checkout is Lightning-first. Check on mainnet.
- [ ] **Keep the amount that actually arrived for Lightning payments** (the order
      amount is what is recorded today; the received amount is only in the log), so
      the accounts match the wallet when a fee applies.
- [ ] **Sign-in flooding.** Enough machines sending junk sign-ins can still make a new
      sign-in slow or refused for a while (signed-in owners are not affected). Accepted
      for now; revisit with real traffic.
- [ ] **Emergency exit costs (the reserve warning was removed on 10 November).** Pulling
      coins out alone, if the Ark server ever stops, costs on-chain fees per coin. On
      mainnet, measure what that costs and check the runbook says so plainly.
