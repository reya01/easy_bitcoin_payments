# Easy Bitcoin Payments

> **⚠ EXPERIMENTAL STATUS — NOT FOR REAL FUNDS.** EBP is pre-release software
> under active development. Its wallet integration has not been audited, its
> response shapes are pinned by fixture tests, not a security audit, and no independent
> security audit has been completed. **Do not connect it to mainnet or real bitcoin.** Use signet or
> regtest only until this notice is removed.

*Free, open-source, self-hosted Bitcoin payments for online stores — built on Ark and Bark.*

> 📱 **Testing EBP?** Follow [docs/MASTER_TEST_WALKTHROUGH.md](docs/MASTER_TEST_WALKTHROUGH.md) — the complete beginner walkthrough (AWS + iPhone, every known error and its fix). It is updated whenever a new failure is discovered.

Easy Bitcoin Payments (EBP) is a single Rust binary plus SQLite that sits between
your store and Bitcoin. No custodian, no middleman, no software fees. Your keys,
your coins, your server.

This repository is at **Phase 1 code-complete** (2026-11-01) and remains
**EXPERIMENTAL — not for real funds**. CI is green; the canonical suite is `cargo test --all --all-features` (**134 passed, 0 failed**).
Every Lightning-side client-facing barkd shape is live-verified against
**barkd 0.8.0 on signet** and pinned in
[`src/wallet/barkd/live_shapes.rs`](src/wallet/barkd/live_shapes.rs) with shape tests;
the on-chain receive history entry shape is still **UNPINNED** and crediting is
fail-closed. Remaining `VERIFY` markers are limited to request/edge shapes the
pinned OpenAPI does not schema. The wallet layer talks to a
[barkd](https://second.tech/docs/barkd_index.md) sidecar (Second's reference
Bark wallet daemon) over its local REST API: Ark + on-chain + Lightning
receive, a polling payment watcher with a persisted cursor, a per-VTXO expiry
queue, and a settlement engine that sweeps the capped till to your own wallet.

**RESOLVED 2026-11-03:** the tester's AWS t4g (aarch64) VPS restart-loop was
caused by the Dockerfile's dependency-cache placeholder binary shipping
instead of the server (a cargo mtime-skip trap; mechanism + fix in
[docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md), which now also carries a build
smoke test guarding against the class). Rebuilt and serving on the VPS.

### Documentation map

- [docs/HANDOFF.md](docs/HANDOFF.md) — one-stop context for a new developer/agent.
- [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md) — open issues and debug procedures.
- [docs/PHASE0_SPIKE_PLAN.md](docs/PHASE0_SPIKE_PLAN.md) — Phase 0 spike questions and log.
- [docs/PHASE1_PLAN.md](docs/PHASE1_PLAN.md) — Phase 1 tasks, status, and close-out.
- [docs/RECIPE_SETUP.md](docs/RECIPE_SETUP.md) — zero-to-live-testnet-invoice recipe.
- [docs/RECIPE_BACKUP.md](docs/RECIPE_BACKUP.md) — backup/restore recipe.
- [docs/UPGRADING.md](docs/UPGRADING.md) — version and upgrade notes.
- [docs/MASTER_TEST_WALKTHROUGH.md](docs/MASTER_TEST_WALKTHROUGH.md) — beginner AWS + iPhone walkthrough.
- [CONTRIBUTING.md](CONTRIBUTING.md) — build, test, and contribution rules.
- [ebp.example.toml](ebp.example.toml) — every config key.
- [LICENSE](LICENSE) — MIT.

---

## What ships today

- **Rust server skeleton** — axum 0.8 + SQLite (rusqlite, bundled), single binary, MIT licensed, CI on main and pull requests.
- **Invoice API** — create / get / list / cancel, authenticated with per-store API keys (hashed at rest).
- **Hosted checkout page** — static, mobile-first, live status polling, plain-language states.
- **Payment watcher** — polls barkd's wallet history with a persisted cursor; address matching, idempotent payment recording, VTXO expiry queue.
- **Signed webhook emitter** — `X-EBP-Signature: t=<unix>,v1=<HMAC_SHA256>` with a replay window.
- **Merchant dashboard shell** — mobile-first info page at `/dashboard` (status
  and store info; it cannot create invoices — use the API, see below).
- **Config** — env vars plus an optional TOML file, with sane defaults.
- **Deployment** — Docker one-liner on a VPS (public IP, DNS, TLS; clearnet HTTPS), plus multi-stage Dockerfile, `.dockerignore`, GitHub Actions CI.

### The wallet layer: the barkd sidecar

Payments run through a **barkd sidecar** — Second's reference Bark wallet
daemon — on the internal network. barkd holds the keys (Custody Level 0: a
capped hot "till" on the server; everything the server sweeps is already
yours). The server authenticates with a bearer token:

```bash
# barkd generates the token on first start and stores it in its datadir.
export EBP_BARKD_TOKEN="$(barkd --datadir /path/to/bark secret show)"
```

The token is env-only (`EBP_BARKD_TOKEN`), lives in the deployment env / a
`.env` file next to the compose file, and is never logged. Rotate it with
`barkd secret refresh` + a restart of both daemons. `--expose-mnemonic` stays
off on the compose service; the setup wizard's one-time backup ceremony runs
a separate temporary barkd WITH the flag, reads `GET /wallet/mnemonic` exactly
once, then removes it — the words are never stored or logged by EBP.

Without a token the server boots on the Phase 0 stub wallet (deterministic
placeholder addresses, no payments observed) so development and tests never
need a running barkd.

---

## Quickstart

Requires Rust 1.82+ (pinned in `rust-toolchain.toml`).

```bash
# 1. Create a store. The API key is printed once — save it.
cargo run -- store create --name "My Shop" --webhook-url "https://shop.example.com/ebp"

# 2. Start the server.
cargo run -- serve
```

The server listens on `http://localhost:8080`.

```bash
# 3. Create an invoice.
curl -sS -X POST http://localhost:8080/api/v1/invoices \
  -H "Authorization: Bearer ebp_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
        "order_id": "wc-1234",
        "fiat": { "currency": "EUR", "amount": "42.00" },
        "expiry_minutes": 20
      }'
```

The response contains `checkout_url`, `ark_address`, `onchain_address`,
`amount_sats`, the locked `rate`, and `expires_at`. Open `checkout_url` on a
phone — that is the customer's page.

```bash
# 4. Read it back (the source of truth for your store).
curl -sS http://localhost:8080/api/v1/invoices/inv_... \
  -H "Authorization: Bearer ebp_sk_..."

# 5. List, filter, cancel.
curl -sS "http://localhost:8080/api/v1/invoices?status=pending&limit=20" \
  -H "Authorization: Bearer ebp_sk_..."
curl -sS -X POST http://localhost:8080/api/v1/invoices/inv_.../cancel \
  -H "Authorization: Bearer ebp_sk_..."

# 6. Health.
curl -sS http://localhost:8080/api/v1/health
```

### With the barkd sidecar (Phase 1)

```bash
# 1. Run barkd (see https://second.tech/docs/barkd/install.md) and get its token.
barkd --datadir ~/.bark-ebp &
export EBP_BARKD_TOKEN="$(barkd --datadir ~/.bark-ebp secret show)"

# 2. Point ebp at it and start the server.
export EBP_BARKD_URL="http://127.0.0.1:3000/api/v1"
cargo run -- serve

# 3. Create the wallet (signet + Second's public ASP). The first-run wizard
#    at /setup does this; the manual curl below is the fallback. If the
#    response contains a mnemonic, write it down and store it offline — EBP
#    never reads it back.
curl -fsS -X POST "$EBP_BARKD_URL/wallet/create" \
  -H "Authorization: Bearer $EBP_BARKD_TOKEN" \
  -H "content-type: application/json" \
  -d '{"network": "signet", "ark": "https://ark.signet.2nd.dev"}'
```

With the sidecar running, invoices get real Ark + on-chain addresses and a
Lightning invoice (best effort, via the ASP gateway), the watcher polls
`/wallet/history` with a persisted cursor, and the settlement engine sweeps
the till to `EBP_SETTLEMENT_DESTINATION` while keeping a 15,000 sat/VTXO
on-chain emergency-exit reserve.

---

## API

All `/api/v1/*` routes except `/health` require `Authorization: Bearer <api key>`.

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/v1/invoices` | Create an invoice (fiat-priced or sats-priced) |
| `GET` | `/api/v1/invoices/{id}` | Fetch one invoice (store-scoped) |
| `GET` | `/api/v1/invoices?status=&since=&limit=` | List invoices |
| `POST` | `/api/v1/invoices/{id}/cancel` | Cancel a pending invoice |
| `GET` | `/api/v1/unmatched-payments` | Late payments that arrived after an invoice expired/closed (needs merchant action) |
| `POST` | `/api/v1/unmatched-payments/{id}/resolve` | Mark a late payment as handled |
| `GET` | `/api/v1/admin/status` | Operator view: counters, settlements, till + exit-reserve (needs `EBP_ADMIN_TOKEN` bearer; 404 if unset) |
| `GET` | `/api/v1/health` | Server, database, wallet, ASP reachability |
| `GET` | `/i/{id}` | Hosted checkout page |
| `GET` | `/i/{id}/json` | Full invoice JSON (fetched once by the checkout page) |
| `GET` | `/i/{id}/status` | Checkout status JSON (polled by the page) |
| `GET` | `/dashboard` | Merchant dashboard (static info shell; no invoice creation) |

### Invoice states

```
pending → paid | underpaid | overpaid | expired | cancelled
underpaid → expired
```

`overpaid` is treated as paid. There is no partial-payment top-up in v1.0: an
underpaid invoice expires and the customer starts a fresh one at the current
rate. Refunds are out of scope for v1.0 — Bitcoin payments are final, and the
checkout page says so.

### Webhooks

Every delivery carries:

```
X-EBP-Event: invoice.paid
X-EBP-Idempotency-Key: inv_...:invoice.paid:61000
X-EBP-Signature: t=1700000000,v1=<hex HMAC_SHA256(secret, "<t>.<body>")>
```

Stores must verify the HMAC with their per-store secret, reject timestamps older
than 5 minutes, dedupe on the idempotency key, and treat
`GET /api/v1/invoices/{id}` as the source of truth before fulfilling.

The webhook emitter performs a single delivery attempt. Retries with backoff,
the delivery log, and one-tap re-send are future work.

---

## Configuration

Resolution order: built-in defaults → optional TOML → `EBP_*` environment
variables. See [`ebp.example.toml`](ebp.example.toml) for every key.

| Variable | Default | Meaning |
| --- | --- | --- |
| `EBP_BIND_ADDR` | `0.0.0.0:8080` | Listen address |
| `EBP_DATABASE_PATH` | `./data/ebp.sqlite3` | SQLite file |
| `EBP_PUBLIC_BASE_URL` | `http://localhost:8080` | Used to build checkout links |
| `EBP_DEFAULT_EXPIRY_MINUTES` | `20` | Rate-lock lifetime |
| `EBP_DEFAULT_CURRENCY` | `EUR` | Fallback currency |
| `EBP_HOT_WALLET_CAP_SATS` | `500000` | Custody Level 0 till cap |
| `EBP_WATCHER_INTERVAL_SECS` | `10` | Wallet poll interval |
| `EBP_LOG_LEVEL` | `info,tower_http=info` | `tracing` filter |
| `EBP_KEY_ENCRYPTION_KEY` | unset | Reserved; at-rest encryption is not implemented yet (schema columns exist) |
| `EBP_CHECKOUT_POLICY_URL` | unset | Refund policy link on checkout |
| `EBP_CHECKOUT_ACCENT_COLOR` | `#f7931a` | Checkout branding |
| `EBP_BARKD_URL` | `http://127.0.0.1:3000/api/v1` | barkd sidecar REST API |
| `EBP_BARKD_TOKEN` | unset | barkd bearer token (`barkd secret show`); env-only |
| `EBP_SETTLEMENT_DESTINATION` | unset | Merchant's Ark or on-chain sweep destination |
| `EBP_SETTLEMENT_INTERVAL_SECS` | `300` | Settlement engine cadence |
| `EBP_CONFIG` | unset | Path to an extra TOML config file (see `ebp.example.toml`) |
| `EBP_RENEWAL_LEAD_TIME_SECS` | `604800` | Renew/settle VTXOs this long before expiry |
| `EBP_LIGHTNING_RECEIVE_ENABLED` | `true` | LN invoice per checkout invoice |
| `EBP_MODE` | `test` | `test` (signet, play money) or `live` — `live` is accepted as a config value but is NOT supported in this EXPERIMENTAL release: never point it at real funds |
| `EBP_RATES_PROVIDER` | `static` | `static` (deterministic, offline) or `live` (median of public APIs) |
| `EBP_RATES_TIMEOUT_SECS` | `5` | Live-rate fetch timeout |
| `EBP_MAX_EXPIRY_MINUTES` | `1440` | Upper bound on invoice lifetime |
| `EBP_WEBHOOK_TIMEOUT_SECS` | `10` | Webhook delivery timeout |
| `EBP_WEBHOOK_ALLOW_PRIVATE` | unset | `1` lets webhook URLs target loopback/private addresses (local dev only; env-only, not in the TOML) |
| `EBP_PUBLIC_RATE_LIMIT_PER_MIN` | `120` | Per-client request limit on public `/i/*` checkout routes (`0` disables) |
| `EBP_TRUST_PROXY_HEADERS` | `false` | Use `X-Forwarded-For` as the rate-limit client key (only behind a proxy that overwrites it) |
| `EBP_SETUP_TOKEN` | unset | If set, first-run wizard actions require it (`X-EBP-Setup-Token`); set it before exposing an unfinished instance to a network |
| `EBP_ADMIN_TOKEN` | unset | Optional admin bearer token (static bearer for now; passkey/TOTP is a v1.0 target) |

---

## Docker (VPS-first)

The supported deployment is a Docker Compose stack on a VPS with a public IP; TLS terminates at your own reverse proxy (automated one-click TLS is a v1.0 gate); home/Tor-only hosting is out of scope.

```bash
docker build -t easy-bitcoin-payments .
docker run --rm -p 8080:8080 -v ebp-data:/data easy-bitcoin-payments
docker run --rm -v ebp-data:/data easy-bitcoin-payments store create --name "My Shop"
```

The image runs as a non-root user, stores its database on the `/data` volume,
and exposes a healthcheck against `/api/v1/health`.

### Full stack with the barkd sidecar

```bash
# 1. Start barkd alone and read its token (generated on first start).
docker compose up -d barkd
export EBP_BARKD_TOKEN="$(docker compose exec barkd barkd --datadir /data secret show)"

# 2. Bring up the whole stack.
EBP_PUBLIC_BASE_URL=https://pay.example.com \
EBP_SETTLEMENT_DESTINATION=ark1your-arke-wallet \
docker compose up -d
```

`docker-compose.yml` runs barkd from a checksum-pinned prebuilt 0.8.0 binary, keeps its datadir on
the `barkd-data` volume, healthchecks it against `GET /ping`, and publishes no
barkd ports — it is reachable only on the internal compose network, and every
`/api/v1` call still needs the bearer token. `--expose-mnemonic` is
deliberately absent.

---

## Development

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --all-features
```

Every module with logic has unit tests: the invoice state machine, API-key
hashing and authentication, webhook HMAC signing and verification, fiat→sats
conversion, database migrations, and the watcher's payment application path.

---

## Security notes

- **API keys are hashed at rest** (SHA-256) and shown exactly once at creation.
- **Secrets are never logged.** Webhook secrets and API keys do not appear in
  log lines or error responses.
- **Webhook secrets are stored in plaintext today** because the emitter must
  sign with them. At-rest encryption using `EBP_KEY_ENCRYPTION_KEY` is a
  documented Phase 1 TODO; the config field and schema column already exist so
  the change is additive.
- **No customer PII is required.** An invoice needs an amount and an opaque
  order reference.
- **Custody, Phase 1 (Level 0).** barkd holds the keys on the server; the
  server's bearer token can spend the capped hot float but cannot touch
  anything already swept to your wallet. Sweeps go out automatically on the
  hot cap and ahead of VTXO expiry, and the engine refuses any sweep that
  would breach the 15,000 sat/VTXO on-chain emergency-exit reserve. A
  compromised server can steal at most the float.

---

## License

MIT. See [`LICENSE`](LICENSE).

## EBP vs. BTCPay Server: an honest comparison

EBP and BTCPay Server share a mission: self-hosted, self-custodial, free Bitcoin
payments with no middleman between you and your customers. They differ in the
generation of technology underneath. BTCPay is a first-generation implementation:
Lightning channels, a mandatory full node, and a C#/.NET codebase with roughly
seven years of accretion. EBP is a second-generation attempt: Ark instead of
channels, a single Rust binary instead of a monolith, and a design target of a
merchant who has never opened a terminal. Both positions carry real tradeoffs.
This section states them bluntly, including where EBP loses.

| | BTCPay Server | EBP |
| --- | --- | --- |
| Stack | C#/.NET monolith, ~7 years of accretion | Rust, single binary + SQLite |
| Payment rails | Real Lightning channels + on-chain | Ark via a barkd sidecar; Lightning via the ASP gateway; on-chain fallback |
| Inbound liquidity | Required — the #1 Lightning merchant pain | None — Ark receive is passive |
| Bitcoin node | Full node required | None |
| Ops weight | Heavy: chain sync, channel management, multi-service updates | One binary + one sidecar; no chain sync |
| Wallet model | Server-held Lightning keys | Capped till + automatic sweeps — hot by design, and the UI says so |
| Setup | Sysadmin-grade | One-click deploys + AI-assisted recipe |
| Lightning trust | Zero third party on the payment path | ASP dependency + gateway trust window until round refresh |
| Feature breadth | POS, crowdfunding, pay buttons, plugins | Invoice API, checkout, store connectors |
| Maturity | Thousands of merchants in production | Pre-release |
| Management | Desktop-first | iPhone-first |

### Where BTCPay wins — stated honestly

- **Real Lightning channels.** No third party sits on the payment path. EBP's
  Lightning receive runs through the ASP's gateway and carries a trust window
  until the received VTXO is refreshed in a round, and Ark itself depends on
  the ASP for liveness and coordination. On trust-minimization, BTCPay wins.
- **Battle-tested maturity.** Thousands of merchants, years in production,
  edge cases found and fixed. EBP is pre-release — barkd's response shapes
  have fixture tests, but no independent audit yet.
- **Ecosystem breadth.** In-person POS app, crowdfunding, pay buttons, a
  plugin system. EBP ships an invoice API, a checkout page, and store
  connectors; POS-lite is v1.x.

If you are a trust-minimization purist, use BTCPay.

### Where EBP wins

- **No node, no channels, no inbound liquidity.** Nothing to sync, nothing to
  open, nothing to rebalance. The #1 operational barrier to accepting
  Lightning payments does not exist here.
- **Setup for people who have never opened a terminal.** One-click deploys
  are a spec requirement, not an afterthought, with an AI-assisted recipe
  on top.
- **iPhone-first management.** Payments, settlement, the expiry queue, and
  health alerts are designed to be handled from a phone. If a flow cannot
  be completed on a phone, it does not ship.
- **Smaller breach blast radius.** The server holds a capped till (default
  500,000 sats) that is swept to your own wallet automatically — the UI
  calls it "cash in a till; we empty it into your vault automatically." A
  compromised server can steal the float, not a full Lightning node's hot
  wallet.
- **A modern, maintainable codebase.** One Rust binary and a SQLite file are
  easier to audit, reason about, and patch than a large C# solution stack.

### Who should pick which

Pick **BTCPay** if you want maximum trust-minimization on Lightning, need
in-person POS or crowdfunding today, or already run a node and are comfortable
as a sysadmin. If you enjoy running a full node, BTCPay is the better tool
for you.

Pick **EBP** if you run (or want to run) an online store, have no node and do
not want one, do not want to manage channels, value running the whole
operation from an iPhone, and accept the ASP and gateway trust windows
documented in the spec.

EBP trades protocol purity for accessibility on purpose — [spec.md](spec.md)
documents every tradeoff in full.

### Incidents inform design

In August 2026, a BTCPay Server vulnerability let unauthenticated attackers
read LND admin macaroon files, and they drained merchants' Lightning channels
(foundationdevices.com and others confirmed losses). The failure class:
a high-value signing credential reachable through the web tier, guarding a
hot wallet whose size is a *requirement* of Lightning (channels must hold
liquidity). EBP is designed against that failure class:

- The server-side wallet is a **capped till** (default 500,000 sats) that is
  swept automatically to your own wallet. A compromised server exposes the
  float, not the vault.
- **You do not have to trust the ASP with your funds — only for liveness.**
  Ark is self-custody: you hold the keys to your VTXOs. The Ark Service
  Provider coordinates payment rounds and keeps the service responsive; it
  cannot spend your money. If the ASP disappears or misbehaves, you take
  your funds home with a unilateral exit — slower, but yours. The worst an
  ASP can do to you is *stall* you, not steal from you.
- The barkd bearer token is the one server credential, bound to loopback,
  never logged, and its blast radius is the till cap — not a full Lightning
  node's hot wallet and channels.

The honest counterweight: EBP is unaudited, pre-release, and adds the ASP
liveness dependency. To be precise about the security claim: **we do not
think EBP is more secure than BTCPay Server.** We think it is **lower risk,
on purpose** — an intentional tradeoff that puts a smaller amount of money
in harm's way and constrains what the exposed components can do. A smaller
attack surface with a smaller prize is not the same thing as a stronger
vault; it is a different bet, made deliberately and documented in
`spec.md`.

