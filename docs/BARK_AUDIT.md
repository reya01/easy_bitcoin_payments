# Bark documentation audit (2026-11-05)

What this checks: our barkd client and operating behavior against the pinned
OpenAPI spec (`docs/barkd-openapi.json`, barkd 0.8.0) and the Bark docs in
`docs/bark-reference/`. Nothing here was tested against a live daemon; the
signet drill (`docs/SIGNET_DRILL.md`) is what confirms the real responses.

## 1. Request/response contract (now enforced in CI)

The pinned spec is fully populated (schemas, required fields, deprecations),
but the code comments still called it "empty". `scripts/check_barkd_drift.sh`
only compared paths and HTTP verbs, which is how earlier wrong field names
(R15) got through.

New: `docs/barkd-contract.json` lists, for each of the 18 endpoints we use,
the fields we send, the query parameters, and the fields we read.
`scripts/check_barkd_contract.py` (run in the "barkd openapi drift check" CI
job) fails if a field we send does not exist, a field the spec requires is
missing, a query parameter is wrong, a field we read is not in the 200
response, or an endpoint is deprecated. It was shown to fail on each of those
cases before being trusted.

Result of the audit: **0 field mismatches in what we send and read**, one
latent defect fixed: `CreateWalletRequest.chain_source` was a string while the
spec wants an object (`bitcoind` or `esplora`). Unused, so it never reached
barkd; removed so it cannot be sent wrongly. Stale "VERIFY: schema is empty"
comments were replaced with pointers to the spec and this check.

Still tolerated on purpose until the drill proves the real daemon matches the
spec: the alias fallbacks in `flex::*` and a few serde aliases (for example
`asp_connected`, `payment_request`). They are not in the spec and can be
removed after the drill.

## 2. Behavior against the docs (what already matches)

- VTXO lifetime (`learn_lifetime.md`): renewal refreshes VTXOs inside a
  7-day window (28-day boards/refreshes). Lightning-received VTXOs live 1-3
  days, so they are always "due" and are refreshed on the next 5-minute pass.
- Ark send is impossible after expiry, so renewal runs before any sweep.
- Offboarding is used for withdrawals; emergency exits are never automated
  (`learn_offboard.md`, `learn_exit.md`).
- On-chain exit reserve of 15,000 sats per VTXO (`learn_exit.md`), and a
  sweep that would breach it is refused.
- Recovery words are the root backup; the mnemonic endpoint is only enabled
  during the ceremony (`backups.md`).

## 3. Gaps found (not fixed here; ranked)

1. **No continuous wallet-database backup (highest).** `backups.md` says the
   seed phrase alone restores the balance only with the Ark server's
   cooperation and loses payment history and in-progress exit state; it
   recommends backing up the wallet database after every state change, to
   storage that outlives the host, encrypted with a key derived from the
   mnemonic. Today: `scripts/backup.sh` is a manual, Docker-Compose-only
   script (not usable in the single-container beginner install), and the
   pre-upgrade copies cover only EBP's own database, not barkd's data. For a
   beginner the practical answer is host-level disk snapshots (Render and most
   VPS providers offer them) plus an in-app encrypted download of the wallet
   data; both are future work. Until then, KNOWN_ISSUES says plainly that the
   recovery words are the only complete safety net.
2. **Exit reserve is a fixed constant.** The spec offers `GET /exits/fee`
   and `GET /fees/onchain` for a live estimate; the docs say exit cost
   depends on the transaction-tree depth. Using the live endpoint would make
   the reserve accurate. Needs the live response shape first.
3. **Received VTXOs inherit their original expiry** and the server accepts
   them right up to expiry (`learn_lifetime.md`), so a customer's payment can
   arrive nearly expired. Our 5-minute pass plus the next round is usually
   enough, but there is no explicit warning. The planned Home health
   checklist should show "coins nearing expiry" using `needs_refresh_sat`
   from `/wallet/balance`.
4. **Refresh success is taken from the HTTP reply.** Refreshes complete in a
   later round; the reply carries `status` (pending / confirmed / failed).
   A failed round is retried because the VTXO stays due, so funds are not at
   risk, but the dashboard could surface repeated failures.
5. **Delegated refresh and co-signers are unavailable** per `learn_rounds.md`;
   we do not use them. Nothing to do.

## 4. After the signet drill

Confirm the contract against real responses, then delete the tolerant
aliases, and decide gap 2 with a real `exits/fee` response in hand.
