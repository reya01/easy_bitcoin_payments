# Release candidate checklist

> For the first release meant to be tried with real money, start with
> [MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md): what is confirmed on
> mainnet, what is not, and the decisions left. This file still holds the release
> steps and the signet-era notes.

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
| Browser end-to-end | Setup wizard, recovery-words quiz, admin login, Ark payment, no on-chain way to pay, outage and a hung wallet, restart, underpay, test payment, encrypted backup, payout change, every request matches barkd's pinned API spec |
| WooCommerce end-to-end | A real WordPress + WooCommerce: checkout redirect, forged notice refused, partial and full payment, repeat notice ignored, return link, shop-side tripwire, changed order total not auto-paid |
| Upgrade and restore | Previous build upgraded in place keeps data; encrypted backup, verify (wrong password refused) and restore |
| DigitalOcean rehearsal | The paste-in setup run on a clean Ubuntu 24.04 machine, minus the real internet certificate |
| Scripts and blueprints | shellcheck, cloud-init validity, Render blueprint safety rules |
| barkd API drift | Our client matches barkd 0.8.0's published API |
| Dependency audit | Known-vulnerability scan |
| Docker image | Builds for amd64 (x86-64 only for now; arm64 returns at v1.0) |
| Release dry run (manual) | Builds the binary, packages the plugin, checksums, smoke-tests the binary; publishes and signs nothing |

## 2. What CI cannot prove (a person must do these)

Record the result of each. A box ticked without a real run is worse than an
open box.

- [ ] **Signet drill** (`SIGNET_DRILL.md`): the real barkd, a real Ark server
      and a phone wallet. Biggest gap. Until it is done the first-run wizard
      with the real wallet daemon and the shape of real payments are
      unconfirmed.
- [ ] **DigitalOcean on a real droplet**: paste `deploy/cloud-init.yaml`, wait,
      read the Console, finish setup in a phone browser. Note exactly what the
      Console showed and how long it took. This is the first test of the real
      internet certificate (sslip.io + Let's Encrypt). First run done on 8 November
      2026 (SFO3, signet): see `archive/DROPLET_TEST_RESULTS_2026-11-08.md`. Keep this open
      until the second round (with WooCommerce) is done.
- [ ] **WooCommerce on a real shop host** (not the CI one), with the plugin zip
      from the dry run.
- [ ] **Phone alerts** with the real ntfy app: type or paste the topic into the app, send a test (the QR code was removed;
      the "open in ntfy" tap link is still untried).
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
   and look at it: one `ebp-*.tar.gz` (x86-64), the plugin zip, `SHA256SUMS`.
   Container images count as dependencies too: the Dockerfile base images and
   the Caddy image in `docker-compose.yml` are pinned by digest (CI refuses an
   unpinned one), and Dependabot proposes new digests as PRs. Review those like
   any other dependency update, and do not merge one in the same release as a
   risky change without a dry run.
2. Move the `Unreleased` section of `CHANGELOG.md` under the new version's
   heading (for example `0.2.1-rc1`), and check the version matches the tag in `Cargo.toml`, `Cargo.lock`, and the
   plugin (`ebp-bitcoin.php` header and `EBP_BITCOIN_VERSION`, `readme.txt` Stable tag),
   so Home and the Plugins page show which build is running. The release
   workflow runs `scripts/check_release_version.sh vX.Y.Z` first and stops
   before building if any of these disagree with the tag (you can run it
   yourself first).
3. Tag the new version (for example `v0.2.1-rc1`) on `main` and push it. The
   `-rcN` suffix makes the workflow
   publish a **pre-release**, which GitHub's "latest" and the dashboard's update
   notice both ignore. Install an rc by explicit version, not `latest`.
4. Watch the release run. Then check: assets present, `SHA256SUMS` matches,
   the signature bundle exists, the notes show the image digest and the verify
   command, and the verify command works on your machine.
5. Install that release on a fresh server exactly as `START_HERE.md` says, using
   the digest-pinned image.
6. Only then tell testers. The final release (`v0.2.0`, the number these candidates are
   candidates for) comes after the checklist in
   section 2 is fully ticked; the dashboard will tell rc users when it exists.

## 5. Draft release notes (edit after the drills)

Draft for the first release candidate that has been tried with real money. The
facts behind each line are in [MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md).

> **v0.2.1-rc2 — release candidate: update from the Home tab**
>
> Easy Bitcoin Payments takes Bitcoin payments (Lightning and Ark) into a wallet
> you control, with a WooCommerce plugin and a no-terminal DigitalOcean install.
>
> Play money (signet) is the default. Real bitcoin can be switched on
> (`EBP_MODE=live`, see `docs/START_HERE.md`) and has been used end to end on a
> small scale: payments over Lightning and Ark, a WooCommerce shop, payouts to
> the owner's own wallet, and a full emergency withdrawal. It is still
> experimental. Use only an amount you can afford to lose, keep payments small
> (the server can cap each one), and read `docs/KNOWN_ISSUES.md` first. The
> emergency withdrawal takes at least a day, needs a little regular Bitcoin on
> the server for fees (at least 1,500 sats), and should pay an address in a
> wallet you hold outside the server (minimum 50,000 sats).
>
> New since rc1:
>
> - **Update from the admin page (experimental).** With the new update helper
>   installed, the Home tab offers a new release. Press Start update and it checks
>   the release first, then shows both version numbers, the signature result from
>   this project's release workflow, the exact image, and links to inspect it on
>   GitHub. Only after you type your admin password (and your code, if two-step
>   sign-in is on) and press Update now is that exact image installed. It never
>   goes backwards and says why whenever it refuses. EBP itself never gets
>   control of the server. Install the helper once with
>   `./scripts/install-update-helper.sh` (new DigitalOcean servers do it for you).
> - **Release candidates are offered as updates** on the Home tab.
> - Behind the scenes: a weekly check against new wallet-software releases, and a
>   check that the server program builds byte-for-byte the same on two machines.
>
> The signed Console update was first used for real on 18 November (rc5 to rc1).
> The Home-tab update has only been tested with stand-in programs and in the
> browser: its first real use is installing the next release from rc2. The
> wallet program stays at barkd 0.8.0; the move to Bark 0.9 is planned for v0.3 (v0.3 is not released until EBP runs Bark 0.9; see [BARK_0_9_REVIEW.md](BARK_0_9_REVIEW.md)).
>
> Runs on ordinary x86-64 servers (the DigitalOcean path); ARM builds are paused until v1.0.
>
> Not included: Shopify, Lightning payouts, refunds, automatic updates.
>
> Install: `docs/START_HERE.md` (no terminal). Verify the download with the
> checksums and signature in this release.

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
- [x] **Ark payout address on a different Ark server.** Done: the setup page flags it and
      sweeps refuse it, with a message naming the right server and wallets. Still to try
      once on signet: an address from another Ark server, and a good one that must say
      "Verified".
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
- [x] **Keep the amount that actually arrived for Lightning payments.** Done: each
      payment keeps the wallet's own figure (`payments.received_sats`) next to what was
      credited to the order, and the payments CSV has a `received_sats` column.
- [ ] **Sign-in flooding.** Enough machines sending junk sign-ins can still make a new
      sign-in slow or refused for a while (signed-in owners are not affected). Accepted
      for now; revisit with real traffic.
- [ ] **Emergency exit costs (the reserve warning was removed on 10 November).** Pulling
      coins out alone, if the Ark server ever stops, costs on-chain fees per coin. On
      mainnet, measure what that costs and check the runbook says so plainly.
