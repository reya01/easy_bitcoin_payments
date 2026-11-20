# Moving barkd from 0.8.0 to 0.9.0 (v0.3): checklist (code done, owner's drills pending)

**Status 20 November 2026:** the code move is done in this draft pull request; the owner's drills (step 7) are what hold the merge. **EBP v0.3 means running barkd 0.9** (owner decision, 19 November 2026). This replaces the old 0.8.1 draft (#245) and the canary issue #276. Read [BARK_0_9_REVIEW.md](BARK_0_9_REVIEW.md) first for the dust change.

## Why a version move is not a one-line change

Since 0.8.1, `POST /wallet/send` pays a bare bitcoin address on-chain, and with no `method` it picks Ark, then Lightning, then on-chain. EBP's Ark sweep used to send only `destination` and `amount_sat`, so a refused Ark payment could have turned into an on-chain payment with fees. Every Ark sweep now sends `"method": "ark"`.

## What the code move found (20 November 2026)

- The 0.9.0 binaries' checksums were computed by CI, not copied, and equal the release's own `SHA256SUMS` (amd64 `840d559d...1eb4`, arm64 `7dd4b94d...d344`). Both Dockerfiles and the mainnet-compat workflow pin them.
- The OpenAPI spec was fetched from the running 0.9.0 daemon (`docs/barkd-openapi.json`). It adds `/exits/presign`, `/exits/remaining` and `/nwc/*`, none of which EBP calls, and changes `ArkInfo` and the exit status shape. All 33 path constants and the 27-endpoint field contract EBP uses pass the contract check with 0 problems.
- `POST /wallet/send` documents `method` (`ark`, `lightning`, `onchain`). `docs/barkd-contract.json` now lists it, so the contract check fails if a future spec drops it.
- Not yet run: the mainnet-compat workflow against the real Ark server, and everything in step 7.

## Steps, in order

1. OpenAPI spec refreshed (done).
2. `"method": "ark"` on every Ark sweep, test and contract updated (done).
3. Binary pin and both checksums in `Dockerfile` and `docker/barkd.Dockerfile` (done, computed by CI).
4. `mainnet-compat-check.yml` moved to 0.9.0 (done). **Run it from the Actions tab before merging.** It must pass.
5. Checks that must be green: the barkd OpenAPI drift check, the contract check, the upgrade test, the signet smoke and pay workflows.
6. **On a test wallet (owner):** run `./scripts/drill_capture.sh --with-debug-info` before and after the move and compare the VTXO xpub. It must be identical. Do one small Ark sweep and confirm it arrives as an Ark payment, not on-chain. Keep that capture private (it holds the xpub).
7. **Emergency-exit re-drill with small coins (owner):** coins under 330, around 1,000 and around 1,500 sats. 0.9 is aimed at the "own output is below the dust limit" failure of drill F9; the old plain message stays as a fallback.
8. **Ask Second:** the date `ark.second.tech` moves to 0.9, the exact upgrade-error text and the threshold (330 sats isolated, about 1,300 sats refused for Bark 0.8.1 or older wallets). Until Second moves, a 0.9 wallet against a 0.8 server must be checked in step 4.

## Not part of this move

Presigned emergency exits and NWC stay out.
