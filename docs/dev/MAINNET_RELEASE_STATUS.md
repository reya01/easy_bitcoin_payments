# First real-money release: where things stand (16 November 2026)

This is the one page to read before deciding to cut the first release that is
meant to be tried with real money. It replaces the signet-only wording in
`RC_CHECKLIST.md` sections 2, 5 and 6 for that release; the rest of that file
(release steps, name and security decisions) still applies. Nothing here
publishes anything: tags, releases and GitHub settings stay with the owner.

No addresses, invoices or transaction ids are recorded here or anywhere in the
repository. Results are described in words.

## 1. Confirmed on mainnet with real money

| What | Result | Where recorded |
| --- | --- | --- |
| Setup on a fresh server in live mode, wizard end to end | Worked | owner's run, 15 Nov |
| Lightning payment, 1,000 sats (admin test payment) | Paid; the server held the full amount, no fee taken | `MAINNET_DRILL_2026-11-15.md`, owner's run |
| Ark payment, 1,000 sats | Paid; full amount held | owner's run |
| Shop link-up (real WooCommerce store), then two small orders, one Lightning and one Ark (about 154 sats each) | Both orders paid and the shop received the payment notices ("Your shop is receiving payment notices") | owner's run, 15 Nov |
| Sweep to the owner's own Ark wallet | 1,000 sats arrived | owner's run |
| Routine upkeep (coin renewal) | Three renewals (1,000, 554 and 754 sats) all completed; the held balance fell by about 5 sats in total | owner's run, 16 Nov |
| Late full payment: a 1,000-sat Lightning invoice paid after its 20-minute countdown ended (the page showed Expired first) | Expired order turned Paid within about two minutes of paying | owner's run, 16 Nov |
| Ark checkout: the new "type this exact amount" box (309 sats), typed into Arké | Box clear and prominent; order paid and the shop was notified | owner's run, 16 Nov |
| Weekly wallet check on a new server; Pause and Resume; test notification; payments CSV; backup download and check | All worked: the check waited until 22 November, pausing refused new payments, the CSV matched every payment, the backup held 6 files | owner's run, 16 Nov |
| Receive fees on Lightning | Receiver pays nothing at any size; EBP's crediting rule is correct | `LIGHTNING_RECEIVE_FEES_2026-11-15.md` |
| Duplicate and late payments, stuck claims, fee ranges, board and offboard sizes | Measured; code changed to match (late full payment now completes) | `SERIES2_TESTS_2026-11-15.md`, `MAINNET_DRILL_RESPONSE.md` |
| Wallet restore from the 12 words, crash recovery | Worked (about a second) | `MAINNET_DRILL_2026-11-15_round2.md` |
| Emergency exit, whole lifecycle on about 99,000 sats | Worked: claimable after about 24 hours at the predicted block; the claim paid the owner's own Bitcoin address directly; total cost about 700 sats (0.7%) | `EMERGENCY_EXIT_2026-11-15.md` |

## 2. Built since those tests, proven by CI and browser tests only

These have not been seen on a live server yet. Each is small, but "tested on
mainnet" does not apply to them:

- Emergency exit: automatic refresh before pricing, the optional "set aside
  fee money" box, the 1,500-sat floor with 5,000 to 10,000 recommended, the
  hard stop when the wallet cannot fund the exit, the outside-wallet wording.
- Amounts in error messages shown in sats, clearer Alerts wording.
- The optional first test payment (setup page and Home; the live server already
  had paid payments, so the Home card stayed hidden) and the phone-alerts
  setup fix (Set up now leads to the steps in Settings).

The next real-money session should look at these: set up phone alerts from
Home after updating; set up a fresh server and look at the test-payment prompt
on its setup page and Home; and, when there is enough on the server, open the
emergency-exit page.

## 3. Still open

Needs a real-money test or an answer from someone else:

- Spending exited coins into a wallet kept on the server needs the wallet's
  database file (not yet tested). The guide now tells owners to send the claim
  to an outside wallet, which avoids this path; the backup guide already says
  the database file, not only the 12 words, carries an exit in progress.
- Lightning claims depend on mailbox notifications; a missed one stayed stuck
  for hours in the drills. Ask Second how claims are issued and whether a
  missed one can be retried.
- Does a refresh clear the "cannot price the exit" error inside EBP's own
  flow (seen by hand, not through the page)?
- Renewal cost looks small: wallet-flagged Lightning coins are refreshed at
  once and a refresh can cost 0 to 213 sats in the drills, but three real
  renewals cost about 5 sats in total. Keep watching; add a cost guard only if
  small coins start losing a real share.
- A first payment on an empty till: kept as a note in the test sheet only.

Decisions for the owner:

- The 100,000-sat floor for emergency exits. The measured total cost was about
  700 sats, so a lower floor is defensible; how low is the owner's call.
- Public name, security reports setting, protected tags, stale branches
  (`RC_CHECKLIST.md` section 3).
- Held pull requests: Dependabot #163, #165, #166 and docs PR #150.

## 4. Suggested wording for the notes of the first real-money release

Replace the signet-only line in `RC_CHECKLIST.md` section 5 with:

> Play money (signet) is the default. Real bitcoin can be switched on
> (`EBP_MODE=live`, see `docs/START_HERE.md`) and has been used end to end on a
> small scale: payments over Lightning and Ark, a WooCommerce shop, payouts to
> the owner's own wallet, and a full emergency withdrawal. It is still
> experimental. Use only an amount you can afford to lose, keep payments small
> (the server can cap each one), and read `docs/KNOWN_ISSUES.md` first. The
> emergency withdrawal takes at least a day, needs a little regular Bitcoin on
> the server for fees (at least 1,500 sats), and should pay an address in a
> wallet you hold outside the server.

Before tagging: move the `Unreleased` part of `CHANGELOG.md` under a new
heading, update the version in the places `RC_CHECKLIST.md` section 4 lists
(`scripts/check_release_version.sh` checks them), and run the manual
**Release dry run**. A server created from `main` already runs these changes;
a server built from an older tag needs `docs/UPGRADING.md`.
