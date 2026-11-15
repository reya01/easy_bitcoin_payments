# Mainnet Live Drill — two barkd wallets, 5,000 sats (2026-11-15)

**Purpose:** verify EBP's barkd integration assumptions against **real mainnet** —
payment rails, fees, timing, error shapes, and edge cases — with real funds at risk.
All findings below were **observed, not inferred**. This file is input for the coding AI.

**Environment:** bark 0.8.0 (`959a489f`), two wallets ("till" :35351, "customer" :35352),
ASP = `https://ark.second.tech/` (Second's **mainnet** production ASP), Esplora =
`https://mempool.second.tech/api`. Wallets created with `bark create --mainnet`.
Budget: 5,000 sats funded by the operator via a BOLT11 invoice paid from the Arké iPhone wallet.

**Scope guard:** this drill was read/execute-only against the wallets; no repo code was
modified. Findings feed `src/wallet/barkd.rs` fixes + `scripts/check_barkd_drift.sh`.

---

## Results at a glance

| # | Test | Result | Time | Fee |
|---|------|--------|------|-----|
| 1 | Lightning receive (ASP gateway BOLT11) | ✅ instant claim — spendable on **first poll (~10s)** | — | 0 to payee |
| 2 | Ark→Ark send (VTXO transfer, `bark.arkoor`) | ✅ **instant, zero fee** (`offchain_fee_sat: 0`) | 1.1s | **0** |
| 3 | Lightning send (payer = wallet with ark VTXOs) | ✅ claimed on next poll (~10s) | 3.4s init | **20 sats on 1,000 (2%)** |
| 4 | Edge: pay an invoice with 5s expiry | ⚠️ accepted & settled — see F4 | — | — |
| 5 | Edge: pay > balance / garbage dest / over-balance ark send | ✅ clean typed errors (see F10) | — | — |
| 6 | Offboard 1,000-sat VTXO → onchain address | ✅ fee **55 sats (5.5%)**, trusted onchain in **~330s** | 2.8s start | 55 |
| 7 | Board 500 sats (onchain→ark) | ❌ rejected: **ASP minimum board = 50,000 sats** (F6) | — | — |
| 8 | Ark-destination send with ONLY onchain funds | ❌ error, **no auto-board** (F5) | — | — |
| 9 | Unilateral exit fee estimate | ❌ fails for Non-Standard VTXOs (F9) | — | — |

## Money ledger

- In: 5,000 sats (Lightning, from operator's Arké wallet)
- Fees paid: 20 + 20 (two Lightning sends) + 55 (offboard) = **95 sats**
- Returned to operator's ark address: **3,460 sats** (instant, zero fee)
- Stranded onchain on customer wallet: **945 sats** (below 50k re-board minimum —
  retrievable by offboarding to a BTC address on request)

## Findings (for the coding AI)

**F1 — Lightning receive is instant on mainnet.** The HTLC was claimed and became a
spendable VTXO on the first balance poll (~10s). EBP's checkout "paid" flow can treat
Lightning receives as immediately spendable — matching the signet drill.

**F2 — Ark→Ark transfers are instant and free.** `POST /wallet/send` returned in 1.1s
and the recipient saw `spendable_sat` on the first poll, with `offchain_fee_sat: 0`.
History subsystem: `bark.arkoor` / `kind: receive`.

**F3 — Lightning fees are payer-side, ~2%.** Both LN legs cost the *sender* 20 sats on
1,000. The `/fees/lightning/pay?amount_sat=N` endpoint returns the exact split:
`{"gross_amount_sat":1020,"fee_sat":20,"net_amount_sat":1000,"vtxos_spent":[...]}` —
EBP should use this to quote fees before paying.

**F4 — Invoice expiry is silently clamped by the ASP (critical for EBP's order flow).**
`POST /lightning/receives/invoice` accepted `expiry_secs: 5` without error, but the
invoice remained payable and settled **minutes later** — the ASP enforces its own
minimum expiry. EBP consequence: an invoice shown as "expired" in the order UI can
still be paid successfully afterward. The order state machine must accept late payment
(fulfill, or mark late-paid) — never treat local expiry as a payment cutoff, and never
treat the API response's expiry as honored.

**F5 — Rail selection: ark destinations consume ark VTXOs first, and NEVER auto-board.**
A 500-sat send to an ark address spent ark VTXOs even though the wallet held 945
trusted onchain sats. With ONLY onchain funds, the same call fails with
`"Insufficient money available. Needed 0.00000900 BTC but 0 BTC is available"`.
EBP consequence: settlement/top-up flows cannot assume onchain funds can reach an ark
destination directly; use the explicit board flow (which has a 50k minimum — F6).

**F6 — ASP minimum board = 50,000 sats.** `/boards/board-amount` and
`/fees/board?amount_sat=` reject below it with a clean 500:
`"board amount ... is less than minimum board amount required by server (0.00050000 BTC)"`.

**F7 — Offboard: 5.5% fee at this size, fast, and the REST shape differs from our docs.**
`POST /fees/offboard` requires `{"address", "vtxos": ["<full_vtxo_id>", ...]}`
(NOT `amount_sat`); `GET /fees/offboard-all?address=...` (query param, not JSON).
Execution: `POST /wallet/offboard/vtxos` → `{"offboard_txid": "...", "movement_id": 5}`
in 2.8s. The destination address saw the funds as `untrusted_pending` at ~45s and
`confirmed`/trusted at ~330s (~5.5 min — faster than the 2-confirmation nominal wait
would suggest). Fee for one VTXO: 55 sats on 1,000.

**F8 — `/offboardings/start` (in our pinned OpenAPI) returned 404 "not found" on the
live 0.8.0 daemon.** The working endpoints are `/wallet/offboard/vtxos` and
`/wallet/offboard/all`. Run `scripts/check_barkd_drift.sh` against this daemon — the
pinned OpenAPI has drift that would have produced dead code paths in EBP.

**F9 — Unilateral-exit fee estimation fails for Non-Standard VTXOs.** `GET /exits/fee`
on the till (holding a Lightning-receive-claimed VTXO) returned 500:
`"Failed to estimate emergency exit fee: Non-Standard VTXO ...: exit chain is not
relayable: the VTXO's own output is below the dust limit for its script type"`.
EBP consequence: the emergency-exit reserve logic (15,000 sat/VTXO rule) assumes exit
is priceable; for LN-received VTXOs pre-refresh it is not. Needs follow-up: test after
a refresh, and decide fail-open vs fail-closed display behavior.

**F10 — Error taxonomy on live daemon (useful for EBP error mapping):**
- invalid destination → `400 {"message": "argument is not a valid BOLT-11 invoice, BOLT-12 offer, Lightning address or LNURL"}`
- missing JSON field → `422 {"message": "Failed to deserialize the JSON body into the target type: missing field \`destination\` at line 1 column N"}`
- insufficient funds → `500 {"message": "Insufficient money available. Needed 0.00999999 BTC but 0.00004000 BTC is available"}` — **amounts are in BTC decimal, not sats**; EBP must parse/reformat for merchant display.
- `POST /lightning/pay` takes **`destination`** (BOLT11/BOLT12/LNURL/LN-address),
  not `invoice` — our earlier client shape was wrong and this drill caught it.

## Session balance states observed (all on mainnet)

`spendable_sat`, `needs_refresh_sat`, `pending_lightning_send_sat`,
`claimable_lightning_receive_sat`, `pending_in_round_sat`, `pending_board_sat`,
`pending_arkoor_send_sat`, `pending_offboard_sat`, `pending_exit_sat` — flat map,
no aggregate field (matches our pinned client).

## Reproduction notes

- Wallets: `bark create --mainnet --datadir <dir>`; daemons:
  `barkd --datadir <dir> --host 127.0.0.1 --port <p>`
- Auth: `Authorization: Bearer <token>`; token = `barkd --datadir <dir> secret show`
  (never log)
- Mnemonics stored with `chmod 600`, never displayed — treat both datadirs as live keys
- All amounts/payments small; ASP = Second's production mainnet service — treat its
  responses as the consensus for EBP integration behavior

## Corrections after coding-AI review (2026-11-15)

The coding AI correctly rejected the F8/F10 framing: **EBP's code never called the
wrong endpoints** — `src/wallet/barkd.rs` already uses `/wallet/offboard/vtxos`,
`/lightning/pay {destination}`, etc. Verification run on the live daemon:

- `scripts/check_barkd_drift.sh` (code ↔ pinned spec): **29 constants, 0 method
  mismatches**, 4 cosmetic warnings (constants missing an HTTP verb in their doc
  comment — `PATH_OFFBOARD_VTXOS`, `PATH_ARK_INFO`, + 2).
- Live-daemon probe (pinned spec ↔ live barkd 0.8.0, 73 paths): all present except
  `GET /api/v1/wallet/debug-info` (genuinely not implemented on the daemon — spec is
  ahead) and `GET /api/v1/wallet/mnemonic` (exists but returns
  `{"message":"Mnemonic endpoint is disabled"}` — intentionally disabled by daemon
  config, a security feature, not drift).
- The real drift is **upstream**: second.tech's docs describe `POST /offboardings/start`,
  which the live daemon does not implement (`/api/v1/offboardings/start` → 404;
  the bare path serves the ASP's web UI). The drill finding stands as documentation
  drift in Second's published docs — worth reporting upstream, not a code bug here.

**Net:** no action items for this repo from F8/F10 beyond the doc-comment verb
warnings.

- Board ≥ 50k sats to verify board fees + round inclusion timing
- Unilateral exit on a refreshed VTXO (F9) — measure real exit cost + duration
- `bark refresh` behavior on LN-received VTXOs (does it make them standard/exitable?)
- High-volume burst: many small ark sends in one round
- Invoice expiry honored at ASP scale (create invoice, wait out the ASP's actual expiry)
