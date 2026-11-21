# Emergency exit drill (2026-11-15) — what worked, what got stuck, what it costs

Environment: bark 0.8.0, till wallet holding one consolidated 99,186-sat standard
VTXO (board-type, refreshed), ASP `ark.second.tech` (mainnet). Real funds; the
exited amount will be claimed to the owner's BTC address after the timelock.
Raw JSON: `exit_fee_final.json`, `exit_start_final.json`, `exit_status_latest.json`
in the drill workspace. READ-ONLY toward the repo; no code changed.

## What was measured

| Step | Result |
|---|---|
| Exit fee estimate (`GET /exits/fee`) on a refreshed/standard VTXO | **works**: `exit_broadcast_fee_sat: 99`, `claim_fee_sat: 17`, `total 116 sats`, `fee_rate 0.128 sat/vB`, `txs_to_broadcast: 2` — but **`fundable: false`** |
| Exit start (`POST /exits/start/all`) | ✅ "Exit started successfully" |
| Branch broadcasts | ❌ **stuck in `awaiting-cpfp-broadcast` for ~2.5h** — never broadcast |
| Root cause (daemon log, verbatim) | `Error creating signed P2A CPFP: ... Insufficient funds: 0 BTC available of 0.00000352 BTC needed` — repeated every ~10s |
| Fix observed | a small on-chain top-up to the wallet (1,000 sats, confirmed) → CPFP built → both transactions broadcast → **both confirmed** |
| Exit completion | ✅ state `awaiting-delta`, confirmed block 976,035 |
| Claim timelock | **claimable_height 976,179 = 143 blocks (~24h) after exit confirmation** |
| Claim | scheduled via `/exits/claim/all {destination: <owner BTC address>}` when the tip reaches 976,179 |
| Exit state across a daemon restart | ✅ **survived a clean SIGTERM + restart** (`pending_exit_sat: 99186` intact) |

## Findings for the coding AI (continuing the F-numbering)

**F17 — An emergency exit needs on-chain funds for its CPFP; a zero-balance wallet
cannot exit.** The exit chain's branch transactions sit below the relay floor
(0.128 sat/vB at the time), so barkd insists on broadcasting them via a CPFP
child — which requires a confirmed on-chain UTXO. With `onchain balance = 0`
(EBP's till design!) the exit never broadcasts; the wallet retries forever.
`GET /exits/fee` warns with `"fundable": false` — **EBP must treat that flag as
a hard gate**: warn the merchant the exit cannot broadcast until the wallet holds
on-chain funds, and ideally keep a small on-chain reserve in the till.

**F18 — The claim timelock after a completed exit is ~24h (143 blocks observed).**
Emergency exits are not instant: exit chain confirmation + 143-block relative
timelock + claim transaction. EBP's "emergency" framing must be honest — this is
a day-scale recovery path, not a fast one. The ASP stays trustworthy-for-liveliness,
not for speed.

**F19 — Exit state survives daemon restarts** (verified with a clean stop/start
mid-exit). A merchant can reboot the server mid-exit without losing the process.

**F20 — Refresh before exit is mandatory in practice:** Lightning-received and
mixed VTXOs made `/exits/fee` fail entirely (F9); after one refresh round
(consolidating 8 VTXOs into 1) the same call returned a real price. EBP's exit
flow should refresh first, then price, then start — in that order.

## Open follow-up (queued)

- **DB-backup necessity test:** Second's docs say spending *exited* bitcoin
  requires the wallet database (a seed phrase alone is not enough). Once the
  claim lands, we test spending the exited funds from a **mnemonic-only
  restore** (no DB). If it fails, EBP's backup story must include the wallet DB
  file — a permanent-loss scenario if the docs/wizard don't say so.

## Claim + completion results (2026-11-16, measured)

| Step | Result |
|---|---|
| Timelock elapsed | claimable at block 976,179; the claim fired at exactly that height (~24h after exit confirmation, as estimated — 143 blocks) |
| Claim execution | `POST /exits/claim/all {destination: <owner BTC address>}` → **"Exit claimed successfully"** |
| Where the funds landed | **directly at the owner's BTC address** (~99,186 sats) — the claim pays the destination, not the wallet; the owner confirmed receipt |
| Exit-side history entry | `bark.exit`, `successful`, `intended = effective = -99,186`, `offchain_fee_sat: 0` (the exit fees were paid by the CPFP funding at broadcast, not as a movement fee) |
| Failed refresh round observed | `bark.round`, `failed`, `offchain_fee_sat: 213` — a refresh attempt burned a 213-sat fee even when the round FAILED (fee not refunded) |
| Wallet DB still required | the exited funds landed at an EXTERNAL address only because we claimed to one; spending exited funds to a wallet-internal address would still require the wallet DB — DB-backup test pending (needs a fresh exit) |

**Complete exit lifecycle cost (measured):** exit fee estimate 116 sats + CPFP
funding 352 sats (consumed) + claim tx fee + failed-round 213 = **~700 sats on
~99,000 sats (~0.7%)**, wall-clock: exit start → both txs confirmed ~40 min
(with CPFP), claim +24h timelock.

**EBP build recommendations (final):** (1) gate the emergency-withdrawal UI on
`fundable` + an on-chain reserve of ≥500 sats; (2) describe the exit as
"~24h to your wallet, non-cancellable after broadcast"; (3) refresh → price →
start, in that order; (4) claim directly to the merchant's external BTC address
when they trigger an exit — it works and skips a hop.
