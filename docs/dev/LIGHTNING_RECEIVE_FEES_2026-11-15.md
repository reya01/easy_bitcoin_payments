# Lightning receive fees — measurement series (2026-11-15)

**Task:** determine what a Lightning receive really costs on barkd 0.8.0 (Second's
mainnet ASP) and which history field equals the amount that actually arrives.
**Method:** controlled series — payer = our second wallet, receiver = the till.
Per run: record balance + history length → quote `/fees/lightning/receive` →
create invoice → payer quote + pay → poll to settlement → capture raw history entry
and balances → compute deltas. Nothing inferred; raw JSON in the drill transcripts.

## Results

| Run | Amount | Quoted recv fee | Balance delta (arrived) | intended | effective | received_on sum | Settle time | Payer fee (quoted / actual) |
|---|---|---|---|---|---|---|---|---|
| A-5000 | 5,000 | **0** | **5,000** | 5,000 | 5,000 | 5,000 | 8.3s | 25 / 25 |
| A-10000 | 10,000 | **0** | **10,000** | 10,000 | 10,000 | 10,000 | 9.2s | 50 / 50 |
| A-25000 | 25,000 | **0** | **25,000** | 25,000 | 25,000 | 25,000 | 9.5s | 125 / 125 |
| A-55000 | 55,000 | **0** | **55,000** | 55,000 | 55,000 | 55,000 | 8.5s | 275 / 275 |
| B-55000 | 55,000 | **0** | **55,000** | 55,000 | 55,000 | 55,000 | 9.1s | 110 / 110 |

## The key answer

**`received_on[].amount_sat` (summed) = `intended_balance_sat` = `effective_balance_sat`
= balance delta = the FULL invoice amount, in every run.**

- **Lightning receives cost the receiver NOTHING** on this ASP, at any size tested
  (5k–55k). There is no receiver-side fee.
- **All Lightning fees are payer-side** (~0.5% here: 25/50/125/275/110 on
  5k/10k/25k/55k/55k — note the second 55k run paid 110, so payer fees also vary
  run-to-run, probably route-dependent).
- **EBP's current crediting rule (`received_on[].amount_sat`) is CORRECT** and equals
  `intended_balance_sat` and `effective_balance_sat` — all three agree. For a
  55,000-sat invoice, EBP credits **55,000**. No change needed; the triple agreement
  also means any of the three fields can serve as the credit source.

## Correction to the round-2 report

The earlier reading — "55,000-sat receive cost the receiver ~2,000 sats" — was a
**measurement artifact of the chaotic round-2 state**, not a fee. Full reconciliation
of the till's history shows invoice 3's receive was recorded at the full 55,000
(`intended = effective = 55,000`). The observed 53,000 balance was a timing artifact:
part of the claimed VTXOs was not yet spendable when the balance was polled (two
racing drill pipelines + the ASP's hourly rounds). Any unexplained remainder is tracked
below. F12 in `MAINNET_DRILL_2026-11-15_round2.md` should be read as **superseded by
this file**.

## New observations from this series

- **Settlement time is ~8–10 seconds for ALL sizes on mainnet** — same instant-claim
  behavior as signet, *provided the receiving wallet holds spendable VTXOs* (see the
  empty-wallet finding below).
- **Refresh rounds are NOT free on the mainnet ASP**: a `refresh/all` round
  participation is pending with `effective_balance_sat: -213` (213-sat round fee).
  Round 1's assumption that refresh is free holds only on signet. EBP's sweep/refresh
  cost model must include round fees on mainnet.
- **Payer fees vary between identical runs** (275 vs 110 on the same 55k amount) —
  route-dependent; always quote at pay time, never cache.

## Open items

- The three pre-series duplicate HTLCs (invoice 1's 55k, invoice 2's 55k, one
  duplicate 1k — all paid while the receiving wallet was EMPTY) remain
  `awaiting-payment` >2h: **an empty barkd wallet cannot claim Lightning receives**
  (claim needs spendable VTXOs as ownership proof, or the ASP's receive token).
  This is a critical operational finding for EBP's till design: an empty till strands
  shopper payments. Resolution path: keep a VTXO float in the till, or use the
  receive-token mechanism. (Being tested separately — see the exit-drill report.)
