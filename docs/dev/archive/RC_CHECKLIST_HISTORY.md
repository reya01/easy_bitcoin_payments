# Release checklist, history (up to 0.2.2-rc1, November 2026)

Moved out of [RC_CHECKLIST.md](../RC_CHECKLIST.md) on 21 November 2026. It is **not current**: the signet-era
to-do lists and the draft notes for 0.2.2-rc1. Many unticked boxes here were done in practice (the signet
drill, the DigitalOcean and WooCommerce tests, phone alerts, the restore drill, the mainnet tests); what is
proven and what is open now is in [MAINNET_RELEASE_STATUS.md](../MAINNET_RELEASE_STATUS.md) and
[KNOWN_ISSUES.md](../../KNOWN_ISSUES.md).

## A. What CI cannot prove (a person must do these)

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

## B. Draft release notes (edit after the drills)

Draft for the first release candidate that has been tried with real money. The
facts behind each line are in [MAINNET_RELEASE_STATUS.md](../MAINNET_RELEASE_STATUS.md).

> **v0.2.2-rc1 — release candidate: send the whole balance, opt-in emergency withdrawal, passkeys (beta)**
>
> Easy Bitcoin Payments takes Bitcoin payments (Lightning and Ark) into a wallet
> you control, with a WooCommerce plugin and a no-terminal DigitalOcean install.
>
> Play money (signet) is the default. Real bitcoin can be switched on
> (`EBP_MODE=live`, see `docs/START_HERE.md`) and has been used end to end on a
> small scale. It is still experimental. Use only an amount you can afford to
> lose, keep payments small (the server can cap each one), and read
> `docs/KNOWN_ISSUES.md` first.
>
> New since rc2:
>
> - **Your money is sent on in full.** Once the server holds the minimum, all of
>   it goes to your payout address in one send, so nothing small is left behind.
>   The minimum is at least 1,500 sats for an Ark payout address and at least
>   50,000 for an on-chain one (every on-chain withdrawal pays a network fee);
>   you can choose more in Settings, and a lower number is refused with the
>   reason. "Move my money now" still sends everything at once.
> - **The emergency withdrawal is opt-in.** Turn it on in Settings, Emergency
>   (needs two-step sign-in). While it is on, at least 5,000 sats of regular
>   Bitcoin stays on the server for its fees (anything above that can be sent
>   back), and you may choose an amount of Ark money to keep there, because a
>   withdrawal needs at least 50,000 sats. Off, everything is sent on. A
>   withdrawal already running always shows and always works.
> - **Passkeys (beta).** Add a passkey (Bitwarden, your phone, a security key)
>   next to the authenticator app, then sign in and approve protected actions
>   with it.
> - **Safer sign-in:** obviously weak passwords are refused; password screens
>   allow 5 tries a minute and 30 an hour; three wrong codes lock further tries.
> - **Checkout:** Lightning by default with a Lightning / Ark choice under the code; EBP no longer takes
>   regular (on-chain) Bitcoin payments.
> - **Server:** the container runs read-only, without root's special powers.
> - Removed: the server settings `EBP_HOT_WALLET_CAP_SATS` and
>   `EBP_ONCHAIN_SWEEP_CAP_SATS` (ignored with a note if still set).
>
> This is the first release meant to be installed with the Home-tab update
> (from rc2 with the update helper installed); the Console update still works.
> The wallet program stays at barkd 0.8.0; the move to Bark 0.9 is planned for
> v0.3 (v0.3 is not released until EBP runs Bark 0.9; see
> [BARK_0_9_REVIEW.md](BARK_0_9_REVIEW.md)).
>
> Runs on ordinary x86-64 servers (the DigitalOcean path); ARM builds are paused until v1.0.
>
> Not included: Shopify, Lightning payouts, refunds, automatic updates.
>
> Install: `docs/START_HERE.md` (no terminal). Verify the download with the
> checksums and signature in this release.

## C. When live mode exists

**Status, 19 November 2026:** live mode exists and has been tried with small real money
(see [MAINNET_RELEASE_STATUS.md](../MAINNET_RELEASE_STATUS.md)). This section is the original plan,
kept for history: an unticked box here is a to-do only if `MAINNET_RELEASE_STATUS.md` or
[KNOWN_ISSUES.md](../../KNOWN_ISSUES.md) still lists it.

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

barkd stays on 0.8.0 until v0.3, which means running Bark 0.9 ([BARK_0_9_REVIEW.md](BARK_0_9_REVIEW.md)).

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
