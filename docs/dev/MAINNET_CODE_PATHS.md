# Real-money (mainnet) code paths: what the server does when something fails

A reading of the code and its tests before the first real-money test, so the test
day starts with known expectations. **Nothing here has run against a real mainnet
wallet yet**; the first run is [MAINNET_TEST_SHEET.md](MAINNET_TEST_SHEET.md).
Each line names where to look in the code.

## Choosing the network (setup wizard)

- Mainnet needs the owner's tick-box, and the server refuses without it
  (`resolve_wallet_choice`, `src/routes/setup.rs`). The Ark server must match the
  network: a "signet" address with mainnet is refused, and Second's mainnet server
  with signet is refused.
- The network is saved with the wallet and the server's mode follows it for good
  (`save_mode`, `wizard_mode`); one restart at the end of setup applies it.
- A wallet that already exists is adopted only if its network matches the choice.
  If the network cannot be read, it is **not** adopted (fails closed).
- After setup, a mode and wallet that disagree show a red line on Home, in both
  directions (`src/health_checks.rs`).
- The emergency-withdrawal lock follows the coins: it applies when the mode is
  live **or** the wallet's network is real Bitcoin (`not_enabled_reason`). It is
  on for mainnet since 15 November 2026 (`ENABLED_ON_MAINNET = true`); the
  lock code stays so the switch can be turned off again.
- Limit: "real Bitcoin" is recognised by barkd's network name `bitcoin` (or
  `mainnet`). If a later barkd renamed it, a real wallet on a server set to play
  money would not be locked by the wallet check; Home would still be loud in the
  other direction.

## Payout address

- Checked three times: when typed in the wizard (`assess`, advice only), when
  saved (`check`, strict: checksum, form, network), and again **at the moment of
  sending** (`settlement.rs`, "last line of defence"). A failure at send time sends
  nothing and shows "payout address refused" on Home.
- The wizard lets the owner tick "use it anyway" for a failing address; the
  server saves it, and the send-time check still refuses it. The setup page's
  danger box now says so on a real-money server (it used to suggest "only testing").
- An Ark address from a different Ark server is refused at send time **only** when
  the server's key can be read and the tags differ. If the key cannot be read,
  barkd's own checks decide. The wizard shows amber "Could not verify" in that case.
- Silent-payment addresses, test-network addresses (`tark1`, `tb1`, `m…`, `2…`),
  and witness versions that anyone could spend are refused.

## Transfers to the owner's wallet

- An automatic Ark transfer needs at least 1,000 sats above the till cap; with the
  default cap of 1,000 the first one happens at about 2,000 sats on the server.
  "Move my money now" ignores both.
- An automatic on-chain withdrawal waits for fees of 50 sat/vB or less, and is
  postponed when the fee rate cannot be read ("never overpay blind").
- A refused or postponed transfer is recorded once per reason, not every cycle.
- A transfer that **fails** (barkd returns an error) is recorded as `failed` on
  every cycle (every 5 minutes) until it works, so a long outage fills Recent
  transfers with repeated rows. Home shows it, and the optional phone alert "An
  automatic withdrawal failed" is sent once and repeats at most daily. The next attempt works from the live balance, so a retry never
  re-sends coins that already left.

## Late and duplicate payments

- The 20-minute countdown is the price lock, not the real limit: the Ark server
  honours a Lightning invoice for 3 hours however short the expiry asked for
  (drill series 2). So a payment of the **full** amount by Ark or Lightning that
  arrives up to 3 hours after the countdown still completes the order (the audit
  log gets `invoice.paid_late`; the shop plugin adds a "check stock" note if it
  had already cancelled the order).
- Still parked under Alerts for you to handle: a part payment after expiry, a
  second payment on an order already paid (never credited twice), anything more
  than 3 hours late, any payment on a cancelled order, and regular-Bitcoin
  payments seen after the deadline.

## Renewal (keeping coins alive)

- Coins are renewed from the server's own terms: inside the free window, never
  later than room for four rounds before expiry, and a fresh coin is left alone
  (only renewed in the last third of its life, the last half when the server's
  rounds are slow). Without the terms the older, more cautious lead time applies.
- Coins under 330 sats cannot be renewed (dust limit). They are skipped, shown on
  Home with the date they lapse, and picked up with other coins when possible.
- A renewal that was requested is judged on later cycles: "requested" until the old
  coins are replaced; counted as failed after 2 hours with no round holding them;
  retried. A coin close to expiry is asked for again, at most once an hour,
  even if a round is supposed to hold it.
- Unreadable balance or coin lists do not stop renewal attempts (tests cover this).

## Emergency exit

- While an exit runs, transfers pause and other coins are still renewed. An exit
  started elsewhere (for example carried in a restored wallet folder) is adopted,
  so Home shows the red line. If barkd cannot say whether one is running and one
  was started, the server stays paused.
- On mainnet the section behaves as on signet: estimate, fee money, the
  50,000-sat floor, password plus two-step code, then Start.
- Fee money (regular Bitcoin on the server): an exit needs about 350 sats of
  confirmed regular Bitcoin to broadcast even when the fee estimate says about
  100 (drill: an exit sat stuck for hours with none). The page asks for at least
  1,500 sats and recommends 5,000 to 10,000 in case fees rise. It can be set
  aside ahead of time (Settings, Emergency; Home shows a line), automatic
  withdrawals never touch it, and "Send leftover to my address" takes it back.
  If barkd reports it cannot fund the exit (`fundable: false`), Start stays grey
  and the server refuses.
- An exit is day-scale: about a day of waiting after the exit transactions
  confirm, then a claim. It survives a server restart.

## Rails and minimums (from the 2026-11-15 drills)

- EBP never boards and never pays an Ark address out of regular Bitcoin funds,
  so the wallet's "no auto-board" behaviour and the server's 50,000-sat
  minimum board cannot hit any EBP flow. Sweeps move whole Ark coins
  (`/wallet/offboard/vtxos`); the on-chain reserve only pays exit fees.
  If a future change adds boarding, it must skip anything under 50,000 sats.
- On the emergency-exit page, coins received over Lightning may not be
  priceable until they are refreshed (barkd answers "Non-Standard VTXO ...
  not relayable"). The page says so in plain words and keeps the Start button
  blocked (it fails closed). Whether a refresh clears it is not yet confirmed
  live.

## What to watch on test day

- The first green or amber verdict on your own `ark1…` address (step 3).
- Whether a transfer ever shows `failed` more than once.
- Whether any coin appears under "too small to renew".
- That the weekly check figures match the phone wallet exactly.
- Whether the very first Lightning payment to a brand-new, empty till completes
  within a minute or two. (Drill series 2: one empty wallet claimed in about 60
  seconds, but three earlier payments to empty wallets stayed unclaimed for
  hours. The cause looked like a missed notification, not emptiness itself, so
  there is deliberately no "your till is empty" warning. If the first payment
  hangs, note the time and tell the coding AI.)
