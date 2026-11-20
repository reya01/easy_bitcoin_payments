# Mainnet Round-2 Drill — board/offboard at scale, restore, double-pay, dup HTLCs (2026-11-15)

Continuation of `MAINNET_DRILL_2026-11-15.md`. Same environment (bark 0.8.0, two
wallets, Second's mainnet ASP `ark.second.tech`). Round-2 budget: 112,000 sats
moved in across multiple payments (see ledger). READ-ONLY toward the repo.

## Round-2 tests + results

| # | Test | Result |
|---|------|--------|
| R1 | **Wallet restore from mnemonic** (fresh datadir, `bark create --mainnet --datadir X --mnemonic <phrase>`) | ✅ **restore + full sync in ~1 second**; balance identical (55,000 sats visible immediately). Restore copy destroyed after test. Merchant-trust critical path PASSES |
| R2 | **Daemon crash recovery** (`kill -9` barkd, restart) | ✅ balance + all history intact after restart; no stuck state |
| R3 | **Wrong-network guard** (mainnet wallet pointed at signet ASP) | ✅ **rejected at wallet creation** ("Failed to connect to provided server") — fail-loud, not silent. Note: creation warns if `db.sqlite` mode >600 — deployment docs should include `chmod 600` |
| R4 | **Board fee at scale** | ✅ **board fee = 0 sats** (`/fees/board?amount_sat=55000` → `fee_sat: 0`); LN receive fee endpoint also quotes 0 |
| R5 | **Double-pay: same BOLT11 paid twice by same wallet** (barkd payer) | ✅ **deduped**: second `POST /lightning/pay` of the same invoice did NOT create a second movement or debit (single `-1000` movement in history) |
| R6 | **Duplicate payment: same BOLT11 paid again by a DIFFERENT wallet (external custodial LN wallet)** | ⚠️ **ACCEPTED and stacked as a second `awaiting-payment` HTLC** — did not error, did not settle instantly. See F11 |
| R7 | **Lightning receive fee at scale** | ⚠️ **RECEIVER paid ~2,000 sats (3.6%) on a 55,000-sat receive** — while `/fees/lightning/receive` quoted `fee_sat: 0`. Small receives (≤5,000) were fee-free. The receive-fee quote endpoint is unreliable at larger amounts (F12) |
| R8 | **Offboard-all at scale** | ✅ 53,000 sats: fee **159 sats**, exec 5s, txid returned; **onchain confirmation took ~8-20 min** (2-conf requirement; signet was ~5.5 min — mainnet pacing is materially slower and variable) |
| R9 | **Board-all (onchain→ark, 52,825 sats)** | ✅ executed + funding tx broadcast; **settlement waited for the next hourly ASP round (~30+ min)** — `pending_board_sat` cleared and VTXOs became spendable at the round |
| R10 | **Ark round cadence on mainnet = 1 HOUR** (`round_interval: "1h"` in `/wallet/ark-info`; signet rounds were ~10s). All round-dependent waits scale accordingly |
| R11 | **Payer-side transient failure**: first 55k payment attempt from a custodial LN wallet FAILED (no HTLC ever reached the ASP); identical retry ~40 min later succeeded. Checkout UX must tolerate failed-then-retried payments |

## New findings for the coding AI (continuing F-numbers from round 1)

**F11 — Duplicate invoice payments from external wallets are ACCEPTED and queued, not
rejected.** An external custodial wallet re-paid an already-claimed 55k invoice; the
ASP created a second `awaiting-payment` HTLC (same for two 1k invoices). Consequences:
(a) a shopper who double-taps pay (or retries after a payer-side failure) can send
MONEY TWICE for one invoice — EBP's order state machine must detect settled-plus-duplicate
HTLCs and refund/handle the excess; (b) the duplicated HTLC does not settle instantly
(still awaiting after 30+ min) — late duplicate settlements may arrive long after the
order was marked paid. **This is the highest-priority finding of both rounds.**

**F12 — `/fees/lightning/receive` under-quotes: quoted 0, actual charge 2,000 sats
(3.6%) on a 55k receive** (small receives ≤5k: fee 0). EBP must not display the quoted
receive fee to merchants as the real net amount; observe actual settlements instead
(history `effective_balance_sat` vs `intended_balance_sat` shows the true fee).

**F13 — Restore is trivial and instant** (R1) — EBP docs can promise "your 12 words
recreate the wallet; funds visible immediately" without caveats.

**F14 — Round cadence on mainnet = 1 hour.** Any EBP flow that waits for a round
(boards, ark-destination sends when spendable VTXOs must be created, exit paths) can
stall up to an hour on mainnet. Checkout "paid" detection is NOT round-bound (Lightning
claims settle in seconds — F1), but settlement-to-spendable sweeps ARE.

**F15 — Daemon crash-safety confirmed** (R2): SIGKILL + restart loses nothing.
EBP's supervisor should still auto-restart barkd, but data risk from crashes is nil.

**F16 — Wrong-network/wrong-ASP wallet creation fails loudly** (R3) — good. Keep the
chmod-600 warning in deployment docs.

## Money ledger (rounds 1 + 2 combined)

- In: 5,000 + 55,000 + 55,000 + 1,000 + 1,000 = 117,000 sats
- Fees observed so far: 2,000 (receive fee, invoice 3) + 159 (offboard-all) +
  20+20 (round-1 LN sends) + 55 (round-1 offboard) ≈ 2,254 sats
- Returned to operator ark: 2,960 + 52,825 = 55,785 sats (round 2 so far) +
  3,460 (round 1) = 59,245 sats
- Still in flight at ASP (awaiting-payment HTLCs, auto-returning on settle):
  ~56,000 sats (duplicate 55k + duplicate 1k + duplicate 1k) + receive fee TBD
- Stranded onchain (customer wallet, below 50k board minimum): 945 sats
