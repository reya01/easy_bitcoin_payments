# Phase 1 — barkd integration plan

**Status:** open
**Depends on:** Phase 0 spike log (docs/PHASE0_SPIKE_PLAN.md) answered in writing
**Window:** months 1–3

## Goal

Replace `StubWallet` with a production barkd-backed wallet layer: a pinned
barkd 0.8.0 sidecar, provisioned and backed up by recipe, receiving on Ark,
Lightning (ASP gateway), and on-chain, with an expiry-aware settlement engine
that sweeps the capped till to the merchant's own wallet (Arké) while keeping
the 15,000 sat/VTXO exit reserve. Proven by a non-technical tester going from
zero to a live testnet invoice using only the recipe and an iPhone.

## Prerequisites

- Phase 0 spike log complete: Q1–Q4 answered with exact versions, commands,
  and observed outputs; its open questions are inputs here.
- `docs/barkd-openapi.json` pinned from barkd 0.8.0 (`GET /api-docs/openapi.json`).
- Second's public signet ASP (`https://ark.signet.2nd.dev`) reachable; a
  regtest ark-server available for CI.
- Phase 0-derived defaults recorded: `hot_wallet_cap_sats`, renewal lead
  time, LN gateway fee table, Arké sweep address format.

## Tasks

### 1. Package the barkd sidecar (pinned 0.8.0)

- **What:** Docker image, docker-compose service, and Umbrel manifest running
  barkd 0.8.0 next to the ebp binary.
- **How:** build from the 0.8.0 source tag or checksum-verified release
  binary; datadir on a named volume; bind to the internal container network
  only (plain HTTP, loopback-grade trust per barkd_start.md); no published
  ports; healthcheck against the status endpoint; image digest pinned in
  compose and in the Umbrel manifest.
- **Done when:** `docker compose up` on a fresh machine starts ebp + barkd;
  barkd is unreachable from outside the internal network; Umbrel install from
  the manifest works on real hardware; version and digest pinned in-repo.

### 2. OpenAPI client generation + CI drift check

- **What:** typed Rust client for barkd's REST API, generated from the pinned
  spec, with CI failing on drift.
- **How:** generate into `src/wallet/barkd_client/` from
  `docs/barkd-openapi.json`; CI job boots the pinned barkd image, fetches
  `/api-docs/openapi.json`, and diffs it against the pinned file.
- **Done when:** CI fails when the spec is stale (proven by a test PR bumping
  the image tag); `BarkdWallet` uses the generated types; regeneration is one
  documented command.

### 3. Wallet provisioning flow

- **What:** first-run wizard step: create the wallet, back up the mnemonic,
  provision and rotate the API token.
- **How:** `POST /api/v1/wallet` with the ASP address (per the pinned spec);
  mnemonic shown once from the create response — `--expose-mnemonic` stays
  off and `GET /api/v1/wallet/mnemonic` is never called; written-down
  confirmation required before receiving is enabled; token from
  `barkd secret show` passed to ebp via `EBP_BARKD_TOKEN` or a root-only
  secret file, never logged; rotation = `barkd secret refresh` + restart +
  update ebp, written as a runbook.
- **Done when:** a fresh install provisions a wallet with no terminal use;
  the mnemonic ceremony blocks receive until confirmed; the rotation drill
  runs in < 10 minutes and cursor-based polling loses no payments across the
  restart.

### 4. Async `WalletAdapter` + `BarkdWallet` implementation

- **What:** the trait goes async; `BarkdWallet` implements it; the server
  wires to real barkd.
- **How:** method semantics unchanged from `src/wallet/mod.rs`;
  `receive_addresses` → fresh Ark + on-chain address endpoints;
  `poll_payments` → payment history mapped to `WalletPayment`
  (`vtxo_id`/`txid`); `balance_sats` → off-chain balance; `health` → daemon
  status + ASP reachability → `AspStatus`. `StubWallet` stays for tests.
- **Done when:** the server runs against signet barkd; mock-HTTP unit tests
  cover `Unavailable` vs `Rejected` mapping; `GET /api/v1/health` reflects an
  ASP outage within one poll interval.

### 5. Receive addresses + payment attribution

- **What:** fresh address per invoice; deterministic attribution; the
  same-ASP caveat handled explicitly.
- **How:** persist `invoice_id ↔ address` for every issued address (Ark and
  on-chain fallback); attribution primary key = address; fallback = amount +
  time window, flagged for review. Same-ASP caveat: a payer on the same ASP
  settles as an intra-ASP transfer — Phase 0 Q1 must confirm it surfaces in
  barkd's payment history with the address intact; if not, match on amount +
  invoice window and label the payment "same-ASP" in the dashboard.
- **Done when:** 20/20 signet payments (≥5 same-ASP, ≥5 on-chain) attribute
  correctly with zero manual fixes; ambiguous payments raise an alert instead
  of silently crediting an invoice.

### 6. Payment watcher over barkd polling

- **What:** the polling loop that drives the invoice state machine.
- **How:** poll payments since a persisted cursor; per-rail confirmation
  rules (Ark: VTXO present; on-chain: N confirmations; LN: claimed VTXO);
  emit signed, idempotent, replay-safe events to the internal outbox (store
  webhook delivery is Phase 2).
- **Done when:** a restart mid-payment loses nothing; duplicate poll results
  never double-fire events; invoice transitions match the spec's state list
  under fault injection (barkd down, ASP down, slow claims).

### 7. Expiry queue from real per-VTXO expiries

- **What:** the dashboard expiry queue, built from actual VTXO expiries, not
  estimates.
- **How:** read the VTXO list with per-VTXO expiry heights from barkd (field
  names per the Phase 0 log); convert to wall-clock "move by" dates; sort by
  urgency; LN-receive VTXOs carry 1–3 day lifetimes and are flagged urgent;
  renewal lead time from Phase 0 (default 7 days, shorter for LN VTXOs).
- **Done when:** the queue matches barkd's VTXO output exactly on signet; an
  LN-received VTXO appears with its real 1–3 day expiry within one poll;
  every row has a plain-language explanation and action.

### 8. Settlement engine: refresh, sweep/offboard, exit reserve

- **What:** automatic VTXO refresh and sweeps to the merchant's wallet, with
  the 15,000 sat/VTXO on-chain reserve enforced.
- **How:** refresh in ASP rounds before expiry (lead time per Phase 0); sweep
  when the float exceeds `hot_wallet_cap_sats` or on the Phase 0 cadence, via
  Ark send to the merchant's Arké address or offboard to their on-chain
  address; track an on-chain CPFP reserve ≥ 15,000 sats × VTXO count,
  separate from spendable balance; unilateral exit (start/progress/claim)
  stays a documented runbook, not automated.
- **Done when:** on signet, a funded till refreshes before expiry with no
  human action; a sweep lands in Arké using the Phase 0 Q4 address format; a
  sweep that would breach the reserve is refused and logged; the exit drill
  completes per the Phase 0 recipe with fees and time recorded.

### 9. Lightning receive via the ASP gateway

- **What:** LN invoices at checkout through the ASP gateway (no swap
  provider).
- **How:** create invoices via barkd's LN endpoint with a wallet-generated
  preimage; show the gateway fee before checkout from the live fee table
  (Phase 0 Q3), never hardcoded; poll until the claimed VTXO appears; route
  its 1–3 day expiry to the urgent queue; handle restart-mid-claim and stuck
  HTLCs per the Phase 0 runbook.
- **Done when:** a signet LN payment from a real wallet becomes a claimed
  VTXO and a paid invoice; displayed fee equals charged fee; `kill -9`
  mid-claim recovers to a credited invoice; failed payments end in a
  plain-language state, never a silent loss.

### 10. Test mode wiring

- **What:** first-class signet test mode across server, dashboard, recipes.
- **How:** one config flag selects the signet ASP + Esplora defaults;
  persistent dashboard banner ("Test mode — not real money"); faucet links in
  the recipe; test invoices excluded from accounting export.
- **Done when:** a deployment flips signet ↔ mainnet with one config change;
  the full UX gate (task 12) runs in test mode; no test data can reach a
  mainnet-shaped export.

### 11. Backup/restore + lost-phone recipes

- **What:** backups of what actually matters, a restore drill, and the
  lost-phone recipe.
- **How:** the mnemonic is the root backup; barkd datadir backup per barkd's
  docs (snapshot guidance recorded in the runbook); restore drill: fresh
  barkd, restore from mnemonic, confirm balance and VTXOs return; lost-phone
  recipe: reinstall Arké, restore the destination wallet from its own seed,
  update the sweep destination — the server is untouched.
- **Done when:** the restore drill recovers the full signet balance on a
  clean machine; a non-technical tester completes the lost-phone recipe in
  < 30 minutes; no backup artifact readable by the AI layer contains the
  token or mnemonic.

### 12. Setup recipe + the non-technical UX gate

- **What:** the plain-text + AI-assisted recipe, and the gate itself.
- **How:** write the recipe (install → provision → mnemonic backup → first
  test invoice); every step completable on an iPhone; the AI layer reads
  docs, never secrets; run the gate with a non-technical tester, unassisted,
  from zero.
- **Done when:** the tester reaches a live, payable testnet invoice using
  only the recipe and an iPhone; every stumble becomes a doc or product fix
  in the same PR, not a waived pass.

## Risks / open questions

- **Address granularity.** barkd 0.8.0 may not issue a fresh Ark address per
  request (Phase 0 Q1). Fallback: amount + time-window attribution with
  review flags — worse UX; must be known before task 5 starts.
- **Same-ASP transfers** may bypass the normal receive path entirely. If
  undetectable, document the limitation and warn at checkout when payer and
  merchant share an ASP.
- **Offboard economics.** Minimums and fees may exceed small floats; sweep
  cadence and cap must absorb this (Phase 0 Q2 numbers decide).
- **Token rotation downtime.** Rotation requires a barkd restart; quantify
  the window and prove cursor polling catches payments made during it.
- **LN gateway limits** (min/max, fees) come from the live ASP, not docs; the
  fee display must be computed at invoice time.
- **Umbrel review timeline** is outside our control; direct Docker ships
  first, Umbrel follows.
- **Upgrade policy.** When do we move off 0.8.0, and who re-runs the spike
  against the new version? Owner needed before Phase 2.

## Phase acceptance gates

- Every task's done-when criteria met, with evidence in-repo (CI runs, drill
  logs, recorded txids).
- Regtest demo: receive → refresh → exit, reproducible from the spike-log
  command list.
- One real signet/testnet sweep to an Arké wallet, txid recorded.
- CI green, including the OpenAPI drift check against pinned barkd 0.8.0.
- Custody statement updated: exactly what the server can and cannot do with
  keys, token, and float at this phase.

## UX gates (from the spec)

- **Principle 6:** no feature ships until a non-technical tester completes
  its flow unassisted — here: zero → live testnet invoice using only the
  recipe and an iPhone.
- **Principle 7:** every operational action (settlement status, store health,
  alerts, expiry queue) completable on a phone screen; a flow that can't be
  completed on a phone isn't done.
- **Principle 8:** every recipe has a plain-text fallback; the AI layer never
  handles secrets.

## Remaining (Phase 1 close-out)

Landed in this pass: sidecar packaging (compose + pinned Dockerfile), the
watcher over `/wallet/history` with a persisted cursor, the VTXO expiry queue
from `/wallet/vtxos`, settlement sweeps (`/wallet/send`, `/wallet/offboard/all`)
with the 15,000 sat/VTXO exit reserve, Lightning receive via
`/lightning/receives/invoice`, and mock-HTTP tests. Still open, by plan task:

- **Task 2 — drift-check CI**: the client is hand-written against the pinned
  endpoint list; the CI job that boots barkd 0.8.0, fetches
  `/api-docs/openapi.json`, and diffs it against `docs/barkd-openapi.json` is
  not wired. Every `VERIFY: field shape` marker in `src/wallet/barkd.rs` is
  closed by that job's first run.
- **Task 3 — provisioning wizard**: wallet creation is a documented curl
  call; the first-run wizard (create wallet, mnemonic ceremony, token
  rotation runbook) is unbuilt. `--expose-mnemonic` stays off and
  `GET /wallet/mnemonic` is never called.
- **Task 4 — async `WalletAdapter`**: the trait is still synchronous behind
  the bridge thread; the async migration is mechanical but unbuilt.
- **Task 5 — attribution drill**: the 20-payment signet attribution run
  (incl. >=5 same-ASP, >=5 on-chain) has not been executed.
- **Task 8 leftovers**: expiry-driven and oldest-VTXO-age sweep triggers;
  partial on-chain sweeps via `/wallet/offboard/vtxos`.
- **Task 9 leftovers**: LN fee display from `GET /fees/lightning/receive` at
  invoice time; stuck-HTLC and restart-mid-claim runbooks.
- **Task 10 — test mode**: the one-flag signet switch and dashboard banner.
- **Task 11 — backup/restore + lost-phone recipes** (mnemonic is the root
  backup; barkd datadir snapshot guidance).
- **Task 12 — setup recipe + the non-technical UX gate.**
- **Umbrel manifest** (task 1's second target).