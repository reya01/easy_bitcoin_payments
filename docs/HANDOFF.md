# Handoff — Easy Bitcoin Payments (EBP)

**Status:** Phase 1 code-complete (2026-11-01), **EXPERIMENTAL — not for real funds**.
`cargo test`: **130 passed, 0 failed**. All client-facing barkd response shapes are
live-verified against **barkd 0.8.0 on signet** and pinned in
[`src/wallet/barkd/live_shapes.rs`](../src/wallet/barkd/live_shapes.rs) with shape tests.
Remaining `VERIFY` markers are limited to request/edge shapes the pinned OpenAPI
does not schema.

**Resolved 2026-11-03:** the tester's AWS t4g (aarch64) VPS restart-loop was
the Dockerfile's dependency-cache placeholder binary shipping instead of the
server (cargo mtime-skip trap; fix + build smoke test in place; rebuilt and
serving on the VPS). Full story: [docs/KNOWN_ISSUES.md](KNOWN_ISSUES.md).

## Read first

1. [README.md](../README.md) — what this is, quickstart, API, config.
2. [docs/PHASE1_PLAN.md](PHASE1_PLAN.md) — Phase 1 tasks and close-out status.
3. [docs/PHASE0_SPIKE_PLAN.md](PHASE0_SPIKE_PLAN.md) — Phase 0 spike questions/log.
4. [docs/RECIPE_SETUP.md](RECIPE_SETUP.md) — zero-to-live-testnet-invoice recipe.
5. [docs/KNOWN_ISSUES.md](KNOWN_ISSUES.md) — open issues and debug procedures.
6. [docs/MASTER_TEST_WALKTHROUGH.md](MASTER_TEST_WALKTHROUGH.md) — beginner AWS + iPhone walkthrough.
7. [CONTRIBUTING.md](../CONTRIBUTING.md) — build, test, contribution rules.
8. [ebp.example.toml](../ebp.example.toml) — every config key.

## Architecture decisions and WHY

- **barkd sidecar, not the Bark SDK.** EBP is a thin Rust client of Second’s
  reference Bark wallet daemon over its local REST API. WHY: barkd is built for
  server/webshop use, keeps wallet state and signing in one audited daemon, and
  lets EBP stay a single Rust binary + SQLite. The SDK would couple EBP to
  Bark’s internal Rust API and force wallet state into the payments process.
- **Capped till, Custody Level 0.** The server holds only a capped hot “till”
  (`EBP_HOT_WALLET_CAP_SATS`, default 500,000 sats) and sweeps excess to the
  merchant’s own wallet. WHY: passive Ark receive needs a server-side wallet,
  but the honest custody story is “minimize, cap, sweep home, document exactly.”
  The 15,000 sat/VTXO on-chain reserve keeps emergency exits fundable.
- **No upstream assumptions.** EBP works with Bark/barkd/Arké as they exist
  today. WHY: the spec explicitly assumes no upstream contributions and no
  upstream roadmap; missing capabilities are designed around inside EBP.
- **VPS-first, no Umbrel.** Supported deploy is Docker on a VPS with public IP,
  DNS, and TLS. WHY: the payment server must be reachable by stores and
  customers; home/Tor-only hosting is out of scope for v1.0.
- **Lightning via the ASP gateway.** Lightning receive is on by default and
  uses the Ark Service Provider’s Lightning gateway. WHY: Lightning wallets
  vastly outnumber Ark wallets today; the gateway is the only LN receive rail
  available without adding a separate node/swap provider. Fees are the payer’s
  and are shown when the live fee table is available.
- **Refunds out of scope for v1.0.** Bitcoin payments are final; the checkout
  page says so. WHY: refunds require a separate custody/approval model and are
  not needed to prove the payment loop.

## Verified live shapes

- Pointer: [`src/wallet/barkd/live_shapes.rs`](../src/wallet/barkd/live_shapes.rs).
- Pinned against: **barkd 0.8.0, signet**, 2026-11-01.
- Covered: wallet mnemonic (`{"mnemonic": "…"}` under `--expose-mnemonic`),
  wallet connected (`{"connected": true}`), Lightning receive fee body, VTXO
  list empty shape (`[]`), and the other client-facing response bodies used by
  the watcher/settlement/setup paths.
- Drift check: [`scripts/check_barkd_drift.sh`](../scripts/check_barkd_drift.sh)
  verifies every `PATH_*` constant against
  [`docs/barkd-openapi.json`](barkd-openapi.json) in CI.

## Drills performed and what they proved

- **Ark sweep drill.** Sent from the till to an Arké signet address; proved the
  settlement engine’s Ark-send path, cap trigger, and destination parsing
  (`ark1…` / `tark1…`) work against live barkd.
- **Lightning checkout drill.** Created a checkout invoice with LN enabled,
  paid the BOLT11 from a signet LN wallet, and watched the claim land as a VTXO.
  Proved the ASP-gateway receive path, BOLT11 attribution in
  `find_pending_by_address`, and the payer-fee behavior (5,020 sent for a
  5,000-sat invoice; payee receives the full invoice amount).
- **Byte Store purchase drill.** End-to-end store purchase against the hosted
  checkout; proved invoice creation, checkout polling, payment attribution,
  state transition, and signed webhook emission.

## Phase plan

- **Phase 0:** [docs/PHASE0_SPIKE_PLAN.md](PHASE0_SPIKE_PLAN.md) — Bark/barkd
  spike questions, commands, and observed outputs.
- **Phase 1:** [docs/PHASE1_PLAN.md](PHASE1_PLAN.md) — barkd integration,
  watcher, expiry queue, settlement engine, setup wizard, live shape pinning.

## What remains

1. **Release cut.** A human chooses and confirms the `v*` tag. Pushing it makes
   `.github/workflows/release.yml` build x86_64 + aarch64 musl binaries,
   `SHA256SUMS`, the GitHub release, and the multi-arch ghcr.io image used by
   `scripts/install.sh`, `scripts/release.sh`, and the recipe.
2. **UX gate.** A non-technical tester must go zero → live signet invoice using
   only [docs/RECIPE_SETUP.md](RECIPE_SETUP.md) and an iPhone.
3. **VPS sign-off.** The restart-loop is resolved (stub-binary root cause,
   fixed + confirmed). What remains on the VPS is the tester finishing the
   first-run wizard (wallet + recovery-words ceremony) and the walkthrough's
   Part 7 test payment — the formal UX gate (also listed as #2).

## Cold-start commands

```bash
cargo test
cargo run -- store create --name "My Shop" --webhook-url "https://shop.example.com/ebp"
cargo run -- serve
curl -sS http://localhost:8080/api/v1/health
docker compose up -d
docker compose logs --tail=200 ebp
```