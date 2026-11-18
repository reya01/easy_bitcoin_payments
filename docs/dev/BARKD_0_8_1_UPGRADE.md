# Moving barkd from 0.8.0 to 0.8.1 or newer (checklist; not done yet)

**Status:** preparation only. The pinned barkd stays 0.8.0 until the owner decides to move
(see `MAINNET_PLAN.md`, "Bark 0.8.1 changelog review"). Do this as ONE pull request that is
**not merged until the owner has run it on a test wallet.** Several steps need things the
code review cannot do (download the real binaries, read the real OpenAPI), so they are
listed as commands.

## Why a version move is not a one-line change

0.8.1 changes `POST /wallet/send`: it now pays a bare bitcoin address on-chain, and with no
`method` it picks Ark, then Lightning, then on-chain. EBP's Ark sweep sends only
`destination` and `amount_sat`, so a mistake could become an on-chain payment with fees.
The fix is to name the method on every Ark sweep. It goes in the same PR as the version
move, because the 0.8.0 spec and daemon were never shown to accept a `method` field.

## Steps, in order

1. **Get the 0.8.1 (or newer) OpenAPI spec** from the release or from a running barkd, and
   replace `docs/barkd-openapi.json` with it. Read the diff. Lightning `state` fields
   becoming enums is expected (same values). Anything else that changes needs a look.
2. **Name the method on every Ark sweep.** In `src/wallet/barkd.rs`, `ark_send` builds the body
   `{ "destination", "amount_sat" }`; add `"method": "ark"`. Update the test that checks the
   body, and add `/wallet/send` `method` to `docs/barkd-contract.json` so
   `scripts/check_barkd_contract.py` checks it against the new spec. Confirm in the spec that
   the field is called `method` and that `ark` is a listed value; if not, stop.
3. **Update the binary pin.** Download the 0.8.1 release assets for amd64 and arm64, compute
   `sha256sum` of each, and change the version and both checksums in `Dockerfile`
   and `docker/barkd.Dockerfile`. Never copy a checksum from a web page; compute it.
4. **Mainnet compatibility workflow.** Change `BARK_VERSION` (and the "0.8.0" in the job name,
   the summary heading and the header comment) in `.github/workflows/mainnet-compat-check.yml`,
   then run it from the Actions tab. It must pass before the PR is merged.
5. **Checks that must be green:** `barkd openapi drift check`, the contract check, the
   upgrade test (previous build to this build), and the signet smoke and pay workflows.
6. **`GET /lightning/receives/{id}`** now answers 404 for an unknown receive. EBP does not
   call it; confirm with `grep -n "lightning/receives" src/wallet/barkd.rs`.
7. **On a test wallet (owner):** run `./scripts/drill_capture.sh --with-debug-info` before and
   after the move and compare the VTXO xpub. It must be identical. Do one small Ark sweep and
   confirm it arrives as an Ark payment, not on-chain. Keep that capture private (it holds the
   xpub).
8. Update `docs/KNOWN_ISSUES.md` and `CHANGELOG.md`, and the version named in
   `MAINNET_PLAN.md`.

## Not part of this move

Presigned emergency exits stay out; see the plan.
