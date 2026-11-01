# Easy Bitcoin Payments

*Free, open-source, self-hosted Bitcoin payments for online stores — built on Ark and Bark.*

Easy Bitcoin Payments (EBP) is a single Rust binary plus SQLite that sits between
your store and Bitcoin. No custodian, no middleman, no software fees. Your keys,
your coins, your server.

This repository is at **Phase 1 — barkd integration**. The wallet layer talks
to a [barkd](https://second.tech/docs/barkd_index.md) sidecar (Second's
reference Bark wallet daemon) over its local REST API: Ark + on-chain +
Lightning receive, a polling payment watcher with a persisted cursor, a
per-VTXO expiry queue, and a settlement engine that sweeps the capped till to
your own wallet. barkd's response shapes are not yet pinned by CI, so every
parser carries a `VERIFY: field shape` marker in `src/wallet/barkd.rs`.

---

## What Phase 0 ships

- **Rust server skeleton** — axum 0.8 + SQLite (rusqlite, bundled), single binary, MIT licensed, CI on every push.
- **Invoice API** — create / get / list / cancel, authenticated with per-store API keys (hashed at rest).
- **Hosted checkout page** — static, mobile-first, live status polling, plain-language states.
- **Payment watcher** — polls barkd's wallet history with a persisted cursor; address matching, idempotent payment recording, VTXO expiry queue.
- **Signed webhook emitter** — `X-EBP-Signature: t=<unix>, v1=<HMAC_SHA256>` with a replay window.
- **Merchant dashboard placeholder** — mobile-first shell at `/dashboard`.
- **Config** — env vars plus an optional TOML file, with sane defaults.
- **Deployment** — multi-stage Dockerfile, `.dockerignore`, GitHub Actions CI.

### The wallet layer: the barkd sidecar

Payments run through a **barkd sidecar** — Second's reference Bark wallet
daemon — on the internal network. barkd holds the keys (Custody Level 0: a
capped hot "till" on the server; everything the server sweeps is already
yours). The server authenticates with a bearer token:

```bash
# barkd generates the token on first start and stores it in its datadir.
export EBP_BARKD_TOKEN="$(barkd --datadir /path/to/bark secret show)"
```

The token is env-only (`EBP_BARKD_TOKEN`), never written to a config file and
never logged. Rotate it with `barkd secret refresh` + a restart of both
daemons. `--expose-mnemonic` stays off: the server never calls
`GET /wallet/mnemonic`.

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

# 3. Create the wallet (signet + Second's public ASP). TODO(Phase 1): the
#    setup wizard does this; until then it is one curl call. If the response
#    contains a mnemonic, write it down and store it offline — EBP never
#    reads it back.
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
| `GET` | `/api/v1/health` | Server, database, wallet, ASP reachability |
| `GET` | `/i/{id}` | Hosted checkout page |
| `GET` | `/i/{id}/status` | Checkout status JSON (polled by the page) |
| `GET` | `/dashboard` | Merchant dashboard placeholder |

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

Phase 0 performs a single delivery attempt. Retries with backoff, the delivery
log, and one-tap re-send are Phase 1 work.

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
| `EBP_KEY_ENCRYPTION_KEY` | unset | At-rest encryption key (**Phase 1**) |
| `EBP_CHECKOUT_POLICY_URL` | unset | Refund policy link on checkout |
| `EBP_CHECKOUT_ACCENT_COLOR` | `#f7931a` | Checkout branding |
| `EBP_BARKD_URL` | `http://127.0.0.1:3000/api/v1` | barkd sidecar REST API |
| `EBP_BARKD_TOKEN` | unset | barkd bearer token (`barkd secret show`); env-only |
| `EBP_SETTLEMENT_DESTINATION` | unset | Merchant's Ark or on-chain sweep destination |
| `EBP_SETTLEMENT_INTERVAL_SECS` | `300` | Settlement engine cadence |
| `EBP_RENEWAL_LEAD_TIME_SECS` | `604800` | Renew/settle VTXOs this long before expiry |
| `EBP_LIGHTNING_RECEIVE_ENABLED` | `true` | LN invoice per checkout invoice |

---

## Docker

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

`docker-compose.yml` pins barkd to the 0.8.0 source tag, keeps its datadir on
the `barkd-data` volume, healthchecks it against `GET /ping`, and publishes no
barkd ports — it is reachable only on the internal compose network, and every
`/api/v1` call still needs the bearer token. `--expose-mnemonic` is
deliberately absent.

---

## Development

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Every module with logic has unit tests: the invoice state machine, API-key
hashing and authentication, webhook HMAC signing and verification, fiat→sats
conversion, database migrations, and the watcher's payment application path.

---

## Security notes for Phase 0

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