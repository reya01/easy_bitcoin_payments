# First real-money release: where things stand (updated 21 November 2026)

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
| Lightning payment, 1,000 sats (admin test payment) | Paid; the server held the full amount, no fee taken | `archive/MAINNET_DRILL_2026-11-15.md`, owner's run |
| Ark payment, 1,000 sats | Paid; full amount held | owner's run |
| Shop link-up (real WooCommerce store), then two small orders, one Lightning and one Ark (about 154 sats each) | Both orders paid and the shop received the payment notices ("Your shop is receiving payment notices") | owner's run, 15 Nov |
| Sweep to the owner's own Ark wallet | 1,000 sats arrived | owner's run |
| Routine upkeep (coin renewal) | Three renewals (1,000, 554 and 754 sats) all completed; the held balance fell by about 5 sats in total | owner's run, 16 Nov |
| Late full payment: a 1,000-sat Lightning invoice paid after its 20-minute countdown ended (the page showed Expired first) | Expired order turned Paid within about two minutes of paying | owner's run, 16 Nov |
| Ark checkout: the new "type this exact amount" box (309 sats), typed into Arké | Box clear and prominent; order paid and the shop was notified | owner's run, 16 Nov |
| Weekly wallet check on a new server; Pause and Resume; test notification; payments CSV; backup download and check | All worked: the check waited until 22 November, pausing refused new payments, the CSV matched every payment, the backup held 6 files | owner's run, 16 Nov |
| Test server, 17 Nov: pause and resume with a code box, a phone test alert, one Ark and one Lightning order through the real shop (about 155 sats each, the paid page showed a mascot), the payments CSV with the new `received_sats` column, Resend, a backup download and check, the "not available right now" message at the shop while paused, the checkout on a phone, and a server restart | All worked | owner's run, 17 Nov |
| Test server, 18 Nov: the first real signed update, 0.2.0-rc5 to 0.2.1-rc1 with `update.sh` (cosign signature checked against the release workflow and tag commit, digest identical to the release notes, version and balance and history unchanged, wallet connected). Then on rc1: the new Settings and Alerts tabs, pause and resume with a code, a phone test alert, one Lightning and one Ark order, a backup download and check, `update.sh --check` for the same version ("already runs") and for the older rc5 (refused as older), and a restart | All worked | owner's run, 18 Nov |
| Receive fees on Lightning | Receiver pays nothing at any size; EBP's crediting rule is correct | `archive/LIGHTNING_RECEIVE_FEES_2026-11-15.md` |
| Duplicate and late payments, stuck claims, fee ranges, board and offboard sizes | Measured; code changed to match (late full payment now completes) | `archive/SERIES2_TESTS_2026-11-15.md`, `archive/MAINNET_DRILL_RESPONSE.md` |
| Wallet restore from the 12 words, crash recovery | Worked (about a second) | `archive/MAINNET_DRILL_2026-11-15_round2.md` |
| Emergency exit, whole lifecycle on about 99,000 sats | Worked: claimable after about 24 hours at the predicted block; the claim paid the owner's own Bitcoin address directly; total cost about 700 sats (0.7%) | `EMERGENCY_EXIT_2026-11-15.md` |

## 2. Built since those tests, proven by CI and browser tests only

These have not been seen on a live server yet. Each is small, but "tested on
mainnet" does not apply to them:

- Emergency exit: automatic refresh before pricing, the optional "set aside
  fee money" box, the 1,500-sat floor with 5,000 to 10,000 recommended, the
  hard stop when the wallet cannot fund the exit, the outside-wallet wording,
  and the lower 50,000-sat minimum.
- Settings tab: every section folded to one compact row (title and a
  one-sentence summary); Phone alerts live on the Alerts tab.
- Phone alerts: the Home "Set up now" button leading to the steps, and the QR
  code removed. Copy-and-paste setup and a test alert worked on a real phone
  (16 November); the "open in ntfy" tap link is still untried.
- The optional first test payment (setup page and Home; the live server already
  had paid payments, so the Home card stayed hidden).
- Amounts in error messages shown in sats, clearer Alerts wording, and a
  checkout fix so the link back to the shop keeps the right wording.
- The brighter look on the checkout page and the admin page (colour, thick
  outlines, bottom tab bar with icons, mint "paid" page; styling only, every
  word and element id unchanged), and the four animated "paid" scenes with
  their built-in, read-only pictures (added after rc4, 17 November). Looked at
  in screenshots against a stub server and covered by the browser tests, not
  yet on a live server or a real phone. Worth a look in light and dark mode:
  Home, Settings, the checkout waiting, paid and expired pages.

- Added after that run, CI and browser tests only: compact Settings rows with Phone alerts on the Alerts tab, one Turn off button for two-step sign-in, Home no longer calling a failed routine renewal a failed withdrawal, and the signed `update.sh` (first real use is v0.2.1-rc1).

The next real-money session should look at these: update the server and look
at the folded Settings tab; set up phone alerts from Home; set up a fresh
server and look at the test-payment prompt on its setup page and Home (a play
money server shows the same screens); and, when there is enough on the server,
open the emergency-exit page.

**Added in 0.2.2-rc1 (19 November 2026), CI and browser tests only; the owner's test of it is the next step:**

- The whole balance is sent once it reaches a minimum, and nothing is kept by default (hard floors 1,500 sats for an Ark payout address, 50,000 for an on-chain one; the owner can raise them in Settings). Open question for the live test: an Ark send of an exact whole balance, with no change coin left, and whether the wallet takes any fee on it.
- The emergency withdrawal is opt-in. While on, 5,000 sats of regular Bitcoin stay as fee money (nothing else is kept back); **Send back what is above 5,000 sats** returns the extra (barkd `/onchain/send`). Not yet run on real money.
- Passkeys (beta): add, sign in with one, approve protected actions; the approval is tied to the signed-in session.
- A transfer never sends coins that are waiting in a renewal round (found in the pre-release review).
- A locked-down container, tighter password limits and a weak-password block.
- The checkout offers only Lightning and Ark; the code for regular Bitcoin payments is gone.
- The update helper's first real use was this update (0.2.1-rc2 to 0.2.2-rc1) on the owner's test server, 19 November 2026.

**Added in 0.3.0-rc1 to rc3 (20 to 21 November 2026), CI and browser tests; the owner's live checks of them are the next step:**

- barkd 0.9.0 (every Ark send says `"method":"ark"`), and the send journal: each transfer is written down first, an
  unclear one stops all transfers until the wallet's history or the owner ("It arrived" / "It did not arrive")
  settles it. Settling from history has not yet been confirmed on a real 0.9 send.
- The keep-an-amount setting is gone: every transfer sends everything that is free, and an emergency withdrawal
  takes everything on the server. Only the 5,000-sat fee money stays.
- The emergency section opens with one "next step"; the update card keeps the Console steps as a folded
  backup. A false "fee money is below what is kept" warning (found on the owner's test server; the check read an
  on-chain balance that is always 0 on this barkd); fixed in rc2.
- rc3 (fixes from a full-repo review): setup only uses a wallet it made itself; stricter transfer, sign-in and
  Lightning checks; no lost shop events; the weekly wallet check covers a skipped week. Details and what was
  declined: the project doc `full-review-validation-2026-11-21`.

**Added in 0.3.0-rc4 to 0.3.2-rc1 (21 to 22 November 2026); rc4 and 0.3.1 were tried by the owner on the test server:**

- Home: "Next round in" for upkeep waiting on the Ark server, the reason a renewal was asked for, a "waiting for the Ark
  server" line and a mempool.space link for an on-chain transfer, and a CSV download of transfers and upkeep (with
  fiat and price columns when a recent price was saved). Settings folds up when you leave it.
- 0.3.2-rc1 (CI only so far): "It did not arrive" is refused while the wallet history shows the transfer pending
  or sent for a different amount; the bare-binary installer checks the release signature; the update helper
  trusts its pinned cosign first; every container image is pinned by digest; a weekly check compares the pinned barkd
  with Second's Docker Hub image, GitLab's checksums and the tag's commit (Second does not sign releases).

## 3. Still open

Needs a real-money test or an answer from someone else:

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

Decided (16 November 2026):

- Spending exited coins into a wallet kept on the server needs the wallet's
  database file and is untested. Closed by wording: the screen and guide tell
  owners to send the claim to an outside wallet, which the real exit test showed
  works directly. The backup guide already says the database file carries an
  exit in progress.
- The emergency-exit floor is 50,000 sats (was 100,000): a full real exit cost
  about 700 sats. Moving money to the payout address stays the better first
  choice.

Decisions for the owner:

- Public name, security reports setting, protected tags, stale branches
  (`RC_CHECKLIST.md` section 3).
- Held pull requests: Dependabot #165, #166, #236, #238 and #265 (not merged with a risky change; each needs a release dry run).

## 4. Suggested wording about real money, for release notes

Use this in place of any signet-only line in the notes of a release:

> Play money (signet) is the default. Real bitcoin can be switched on
> (`EBP_MODE=live`, see `docs/START_HERE.md`) and has been used end to end on a
> small scale: payments over Lightning and Ark, a WooCommerce shop, payouts to
> the owner's own wallet, and a full emergency withdrawal. It is still
> experimental. Use only an amount you can afford to lose, keep payments small
> (the server can cap each one), and read `docs/KNOWN_ISSUES.md` first. The
> emergency withdrawal is off until you turn it on, takes at least a day, needs
> regular Bitcoin on the server for fees (the page asks for at least 5,000 sats
> once it is on), and should pay an address in a wallet you hold outside the
> server.

Before tagging: move the `Unreleased` part of `CHANGELOG.md` under a new
heading, update the version in the places `RC_CHECKLIST.md` section 4 lists
(`scripts/check_release_version.sh` checks them), and run the manual
**Release dry run**. A server created from `main` already runs these changes;
a server built from an older tag needs `docs/UPGRADING.md`.
