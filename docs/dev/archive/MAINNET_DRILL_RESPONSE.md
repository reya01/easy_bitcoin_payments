# Response to the 2026-11-15 mainnet drills

Item-by-item status against `MAINNET_DRILL_2026-11-15.md` and `..._round2.md`.
Nothing here changes custody, security, secrets handling or money-math.

| # | Drill finding | Status in this repo |
|---|---|---|
| 1 | F8/F10 API drift (`/offboardings/start`, `/lightning/pay {invoice}`) | **Not present.** Neither appears in `src/`. The pinned spec (`docs/barkd-openapi.json`) already has `POST /wallet/offboard/vtxos {address, vtxos}`, `POST /fees/offboard {address, vtxos}`, offboard-all with optional address, and `LightningPayRequest` requiring `destination`. EBP does not call `/lightning/pay` at all. `scripts/check_barkd_drift.sh` still needs a run against a live barkd 0.8.0 by the owner. |
| 2 | F12/F3 fee accounting | **Resolved, no code change.** `LIGHTNING_RECEIVE_FEES_2026-11-15.md` measured 5 receives (5,000 to 55,000 sats): receiver fee 0 every time, and `received_on` = `intended` = `effective` = balance change = the full invoice amount. EBP credits the right amount. The earlier "2,000 sats" reading was a timing artifact; F12 is superseded. Lightning fees are payer-side only and vary per payment. |
| 3 | F11/F4 duplicate and late payments | **Safe today, not automatic.** A duplicate settle or a payment after expiry is never credited twice; it is parked in `unmatched_payments` and shown in Alerts (wording clarified in this change). Accepting a late full payment so the order completes is a behaviour change for the WooCommerce plugin and needs your decision. |
| 4 | F14/F7 mainnet timing | Documented in `MAINNET_TEST_SHEET.md`; detection is poll-based, not round-bound. |
| 5 | F5/F6/F9 rail selection | **Open.** Sweep and reserve logic not changed. Board minimum (50,000 sats) and Non-Standard VTXO exit-fee failures need live data to test. |
| 6 | F10 error amounts | **Done here.** `btc_amounts_to_sats` rewrites "0.00004000 BTC" as "4000 sats" in every barkd error shown to the merchant (unit-tested). |
| 7 | Do-not-change list | Respected. |

## New open items from the receive-fee series

- **An empty till may not be able to claim Lightning receives** (three payments to an empty wallet stayed awaiting for more than 2 hours). Needs its own test and a till-design decision (keep a small Ark float, or use the receive-token mechanism).
- **Refresh rounds cost 213 sats on mainnet** (one observed). Check against the renewal logic and `MIN_RENEW_SATS` (330).
