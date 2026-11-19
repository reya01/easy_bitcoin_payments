# Bark 0.9 review: roadmap change and the dust-output change (19 November 2026)

**Decision (owner, 19 November 2026): EBP v0.3 means running Bark 0.9.** v0.3 is not released until the wallet program (barkd) is 0.9. Until then EBP stays on barkd 0.8.0 (the move to 0.8.1 is folded into this one move).

## What we read, and what we could not

- Read in full: the Bark changelog entry for 0.9.0 (released 19 November 2026) and 0.8.1, <https://second.tech/docs/changelog>.
- **Not readable from here: the text, diff and discussion of GitLab merge request !2822, "Prevent dust outputs on checkpoint txs"** (author Steven Roose, branch `fix-dust-checkpoints`, 6 commits, 23 changed files, opened 18 November). GitLab served only its title and counts. Everything below about it comes from the changelog's "Dust and checkpoints" paragraph, which describes the shipped result. **Before any code change that depends on exact thresholds, the owner or Second should confirm them against the MR or the 0.9 release notes** (the changelog points to `CHANGELOG.md` in the Bark repository).

## The dust change in plain words

Every Ark coin has a pre-signed "checkpoint" transaction that must be accepted by the Bitcoin network if the coin ever has to be exited on-chain. A checkpoint with a very small output (dust) is not accepted, so that exit would fail. Bark 0.9 sends any output **below 330 sats** to a separate "dust isolation" transaction, so the checkpoint stays valid. This covers Ark, Lightning and on-chain payments.

Two consequences from the changelog:

1. **The server refuses the old format.** With a 0.9 server, "on Bark 0.8.1 or older, a payment of any size can fail with an upgrade error" when it spends a coin under **about 1,300 sats**. The size of the payment does not matter; the size of the coin being spent does.
2. In the library, Ark payments always use a checkpoint now and misplaced dust is a `Dust` error. (Not used by EBP, which only talks to barkd over its REST API.)

Related 0.9 changes that touch us: an exit counts as confirmed once a block includes it (no more fee-bumping an already confirmed transaction); the on-chain payment fee estimate uses the same coin selection as the payment; a refresh takes the 1,000 soonest-expiring coins and the server refuses larger refreshes from older wallets; barkd's background sync now detects coins forced on-chain by someone else's exit and starts an emergency exit for them; REST exit status `child_txid` is always empty and `origin` is deprecated.

## What it means for EBP (checked against the code on `main`)

| Area | Today | Effect with a 0.9 server |
|---|---|---|
| **Ark sweep** (`ark_send`, `/wallet/send`, amount = the excess) | barkd picks the coins. EBP's till is full of small coins (every small Lightning order makes a coin the size of the order). | **Main risk.** If barkd 0.8.x picks any coin under about 1,300 sats, the send can fail with an "upgrade" error. Nothing is lost (the coins stay), but the sweep to the owner stops. We do not know the date Second moves its server to 0.9. |
| **On-chain sweep** (`offboard_vtxos`, whole coins, largest first) | Small coins are normally left in the till, so this path mostly avoids them. | Probably fine, but not proven: the changelog says "payments" in general. Also see the net-amount point below. |
| **Renewal minimum** (`MIN_RENEW_SATS` = 330) | Coins that add up to less than 330 are skipped (the wallet refuses less). | Matches the new 330 threshold. No change. |
| **Sweep minimum** (`MIN_SWEEP_EXCESS_SATS` = 1,000, automatic only) | 1,000 sats above the amount kept. | Fine for the dust rule, but an on-chain sweep pays a fee out of the amount: a 1,000-sat sweep can land near or under 330 and be refused. "Move my money now" has no minimum at all. Today that shows as a failed sweep with barkd's wording. |
| **Emergency exit pricing** (`fee_cannot_be_priced`) | Drill F9 (15 November): coins received over Lightning could not be priced ("exit chain is not relayable: the VTXO's own output is below the dust limit"). EBP shows a plain message and never starts the exit. | 0.9 is aimed at exactly this class of coin. After the upgrade, pricing and exits may start to work for them, with extra transactions. **Must be re-drilled with real small coins.** The old message stays as a fallback. |
| **Exits that barkd starts by itself** (new in 0.9) | `exit_pause` already adopts an exit nobody started from Settings: renewals and sweeps pause and Home shows the red line. | Already covered by design; add a test that mimics 0.9 and make Home say "your wallet started an emergency withdrawal on its own". |
| **Refresh size** | EBP asks for all due coins at once. | Cap one request at 1,000 coins (matters only for a very busy shop). |
| **Send rail** | `/wallet/send` is called with only destination and amount. | From 0.8.1 the call picks Ark, then Lightning, then on-chain unless `method` is given. Send `"method": "ark"` (already planned in MAINNET_PLAN.md). |
| **REST exit fields** | EBP does not read `child_txid` or `origin`. | No change. |
| **Board funding** (`BoardFundingAddress`) | EBP does not use it. | No change. |
| **Server lifetime check** (mainnet: lifetime more than twice the refresh threshold) | EBP relies on Second's server terms. | barkd refuses a server that fails it; surface barkd's refusal in plain words. |
| **OpenAPI pin and drift check** | Pinned to 0.8.0 (`docs/barkd-openapi.json`). | Refresh the pin at the upgrade; the drift job flags the change. |
| **Server version** | `/wallet/ark-info` (0.8.0) has no server version field. | Cannot warn "server is newer than your wallet" ahead of time; detect it from the refusal instead (below). |

## Guardrails to add (proposed, in this order)

Each is a separate small PR with its own tests. None is built yet.

1. **Plain "update needed" handling.** When barkd reports the upgrade refusal, record it as a wallet-software-too-old problem (not an address problem), stop retrying every tick, raise one Home line and one alert: "Second's server needs a newer wallet program. Your money is safe. Update EBP." Needs the exact barkd error text; capture it from the first real occurrence or from Second.
2. **Explicit `"method": "ark"`** on every Ark sweep (a refused rail must never turn into a different rail).
3. **Net-amount check before an on-chain sweep:** estimate the fee, and if what would arrive is under 330 plus a margin, hold it and say why in plain words, for the automatic and the manual sweep alike.
4. **Cap a refresh request at 1,000 coins.**
5. **Re-drill emergency exit on 0.9** (owner, real small money), with a coin under 330, one around 1,000 and one around 1,500 sats.
6. **Upgrade PR to barkd 0.9** (refresh the OpenAPI pin and contract checks, check every call against the 0.9 REST changes, then signet drill, then mainnet small-money drill). Done before v0.3-rc1 is cut.

## Open questions for the owner

- When does Second move `ark.second.tech` (and the signet server) to 0.9? Until it does, 0.8.0 keeps working; after it, small-coin payments from 0.8.x can fail. If the date is near, the barkd 0.9 upgrade PR moves ahead of the rest of v0.3.
- Can Second confirm the threshold numbers (330 and about 1,300) and the exact upgrade-error text?
