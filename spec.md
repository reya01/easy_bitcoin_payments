# Easy Bitcoin Payments

*A free & open-source payments layer for online stores, built on Ark and Bark. Run a sovereign, Bitcoin-first business from your own server — manageable entirely from an iPhone.*

---

## Mission

Every online business should be able to accept Bitcoin **directly, for free, with no custodian and no middleman** — and running the software should be as easy as installing an app. Easy Bitcoin Payments (EBP) is free, open-source software any merchant can self-host, connecting their store to Bitcoin's newest payment rail (Ark) with an on-chain fallback, while keys stay in the merchant's hands and the whole operation fits in their pocket.

BTCPay Server's mission, rebuilt on a modern stack for the Ark era: **one-click deploys, iPhone-first management, and AI-assisted setup.**

### What we claim, precisely

This project makes strong claims. Here is exactly what they mean:

- **"No custodian"** describes the destination of funds: everything you earn settles to your own keys. The server's own custody is minimized by design and documented exactly (see *Custody & key management*). Where v1.0 must hold a limited hot float, we say so, cap it, and sweep it home automatically.
- **"No middleman"** describes the software: no company sits between you and your customer, no account can be frozen, no one takes a cut. The underlying rails have their own service relationships — Ark depends on an Ark Service Provider (ASP) for liveness and coordination; a Lightning receive rail (v1.0, via the ASP's Lightning gateway) uses gateway fees; the fee display is a Phase 1 close-out item. These dependencies are documented, monitored, replaceable, and never operated by us.
- **"Free"** means no software fees, ever. Bitcoin miner fees, ASP fees, and Lightning gateway fees exist; EBP never adds to them.
- **"Non-custodial even if compromised"** is scoped: a compromised server cannot touch funds already settled to your wallet, and at Custody Level 1 cannot spend at all. The realistic server-compromise attack is *payment redirection* — showing an attacker's address at checkout — and the threat model addresses it directly.

## Principles

1. **Sovereign by default — and honest about the path.** Funds settle to the merchant's own keys. Server-side custody is a ladder we climb down, not a fact we hide: each release documents exactly what the server can and cannot do. No account can be frozen, no gateway can deplatform you, no processor can leak your customer list — because there is no processor.
2. **Bitcoin only, merchant's choice beyond.** The reference implementation accepts Bitcoin exclusively — that's the point. Bitcoin-preferred setups (a discount for paying in Bitcoin, Bitcoin-first checkout with optional fiat fallback) are supported as opt-in configuration, never defaults.
3. **Free forever, open forever.** MIT-licensed. No hosted middleman tier, no software fees, no transaction fees to anyone. Infrastructure fees that exist outside this software (miner fees, ASP fees, swap fees on opt-in rails) are shown in-product before they are incurred. The ASP is the only standing service relationship, and merchants can run their own.
4. **Easy is the product.** As simple to run as a consumer app. One-click deploys are always the goal — never an afterthought. A merchant who has never opened a terminal should succeed.
5. **Modern stack, boring operations.** Rust core, SQLite by default, single binary, well-documented. No C# monolith, no obsolete-protocol baggage, no node-management burden.
6. **Non-technical by design.** Every flow is designed for someone who has never touched a terminal: plain-English settings, sensible defaults, human-readable errors that say what happened and what to do next, and jargon translated on sight. Where Bitcoin concepts are unavoidable (keys, sweeps, expiry), the UI teaches with plain words and never requires understanding internals to succeed. **No feature ships until a non-technical tester completes its flow unassisted.**
7. **Manage everything from an iPhone.** The full business loop — wallet, payments, settlements, store health — must work end-to-end from a phone. [Arké](https://arke.cash/) (the design-led, open-source iOS Bark wallet built in SwiftUI over Bark's FFI bindings) is the reference destination wallet: sweeps land in Arké, payments are testable from it, and alerts surface in the dashboard's mobile-first PWA. The spec assumes **no upstream contributions and no upstream roadmap**: EBP works with Bark and Arké exactly as they exist today, and where a capability is missing, EBP designs around it within its own codebase — never waiting on, or depending on, another project's future work.
8. **AI-assisted, not AI-dependent.** Setup and operations are driven by a first-class, tested setup wizard inside the product. On top of it sits a guided, AI-assisted recipe: an agent (local, or the merchant's preferred assistant via shareable runbooks) walks non-technical users through install, store connection, first test payment, and troubleshooting. The AI layer reads the docs, not the user's keys; it never handles secrets; every recipe has a plain-text fallback so the software remains fully usable without AI.

## What it is

A single self-hosted service ("the payments server") that sits between an online store and Bitcoin:

```
Store (WooCommerce at v1.0; Shopify, custom, storefront mode later)
   │  order → POST /api/v1/invoices (per-store API key)
   ▼
Easy Bitcoin Payments server — self-hosted, one Rust binary + SQLite
   ├── Invoice API — invoices paying to the merchant's wallet;
   │   fiat-priced with a locked BTC rate; per-store API keys
   ├── Checkout page — static, mobile-first; Ark QR with on-chain
   │   fallback; live status; plain-language states
   ├── Payment watcher — adapter over wallet/ASP state (polling today;
   │   subscription when ASPs expose one); per-rail confirmation rules;
   │   signed, idempotent, replay-safe webhooks to the store
   ├── Settlement engine — expiry-aware automatic sweeps of the server
   │   wallet's capped float to the merchant's own designated wallet
   │   (e.g., their Arké address or a cold on-chain address)
   └── Merchant dashboard — mobile-first PWA; email + in-dashboard alerts;
       expiry queue; accounting export
   ▼
Bark wallet ↔ barkd sidecar → ASP (Second's public Ark server, or the merchant's own Ark server)
   └── VTXOs received → swept to the merchant's own keys (Arké on iPhone)
```

Key design decisions:

- **Receive is passive; settlement is automatic.** Ark's passive receive means accepting a payment requires no merchant action and no interactive signing at checkout. Holding and spending VTXOs *do* require signing on a schedule (see *VTXO lifecycle*); the settlement engine automates that schedule from the server's capped till, sweeping to the merchant's designated wallet. Phone signature approval is not assumed — if the ecosystem ever enables it, it becomes an optional enhancement.
- **iPhone is the admin console.** The dashboard is mobile-first; every operational action (check settlement status, check store health, read alerts) is completable on a phone screen. If a flow can't be completed on a phone, it isn't done.
- **barkd sidecar, ASP-agnostic.** EBP's wallet layer is a thin Rust client of a **barkd sidecar** — Second's reference wallet daemon, built for exactly this use case (webshops, payment processors). Works with Second's public ASP out of the box; supports any Bark-compatible ASP including the merchant's own Ark server (advanced guide). Every client-facing response shape is live-verified against barkd 0.8.0 on signet and pinned in-code with verbatim fixture bodies and shape tests (the daemon's published OpenAPI carries no response schemas, so shapes are captured from the running daemon; the set was completed 2026-11-01 with the mnemonic, connected, and Lightning-fee bodies). ASP status is shown as a traffic-light indicator (fine / degraded / down, with a plain-language explanation); the unilateral-exit runbook is one tap from any ASP alert.
- **One checkout, all rails.** v1.0 checkout covers Ark (instant, cheap, self-custodial), Lightning (via the ASP's fee-labeled gateway), and on-chain in a single screen — customer reach is the product, and Lightning wallets vastly outnumber Ark wallets today. AS BUILT (2026-11-02 drill): the checkout defaults to a combined BIP321 QR (bitcoin:?amount=…&lightning=…&ark=…) with per-rail tabs as the degraded mode; the earlier rail-selector default is superseded. Lightning receive is on by default; the gateway fee estimate is fetched at invoice creation and shown when available (Phase 1 close-out).
- **Store connectors are thin clients.** The WooCommerce plugin calls the merchant's own server and verifies webhooks. No third-party data flows.
- **Standalone storefront mode (v1.x).** A minimal built-in catalog + cart + checkout for Bitcoin-only businesses that want no platform at all — deferred until the core payment loop is proven with real merchants.
- **One-click deploys, as a spec requirement.** Target platform: **Docker one-liner on a VPS** — the only supported path, because reachability is solved there (public IP, DNS, TLS) and the whole product is manageable from an iPhone. "One-click" includes *reachable*: TLS, domain, and store→server callbacks are part of the flow. A release is not "done" until its one-click path — including upgrades from the previous release — is tested.

## Custody & key management

The hardest engineering problem in this project, specified honestly.

**What Bark can do today (the constraint).** Bark is a wallet: it holds keys and manages VTXOs. As of this writing it does not expose a supported watch-only/xpub server mode or an external-signer API, and Arké does not yet expose a "server proposes, phone approves" signing interface. VTXO scripts are not plain xpub derivations. We do not claim otherwise anywhere in product, docs, or marketing.

**The custody ladder.** EBP is built against three defined custody levels:

- **Level 0 — Capped hot wallet on the server (works on Bark today).** The server runs a dedicated Bark wallet ("the till"). Guardrails: a configurable holding cap (default 500,000 sats); automatic sweeps to the merchant's own address on cap, on schedule, and ahead of VTXO expiry; the datadir is access-restricted and continuous backups (bark-backupd, encrypted with a mnemonic-derived key) are supported; guided, confirmed backup at setup. Honest UI label: "your register — like cash in a till; we empty it into your vault automatically." A compromised server can steal at most the float; it cannot touch swept funds.
- **Level 1 — Watch-only server, external signing (aspirational, not assumed).** The server holds only public material and the merchant signs settlement externally (phone or hardware signer). This is attractive but **requires capabilities Bark and Arké do not expose today**. The spec assumes they never arrive; if they do, Level 1 becomes a natural upgrade. EBP does not lobby, wait on, or plan around upstream changes.
- **Level 2 — Restricted server signer (research).** A server-side key limited by Ark script/policy to *renew* VTXOs or *exit to the merchant's fixed address only* — compromise could force funds home but never steal them. Feasibility depends on Ark script capabilities; tracked as an open question.

**Decision.** v1.0 ships **Level 0 unconditionally** — no dependency on upstream changes, no phone-signing requirement (phone signature approval is a nice-to-have if the ecosystem ever supports it, never a spec requirement). The till model with caps and automatic sweeps is the design, and the guardrails above are what make it honest.

**Phone loss.** Losing the iPhone must never mean losing the business. Setup includes a guided, confirmed Arké seed backup; the server's capped float bounds what is ever at risk; restoring the wallet on a new phone resumes settlement. A "lost phone" runbook ships in the docs and the AI recipes.

## VTXO lifecycle & settlement policy

Receiving is passive; holding is not. VTXOs expire and must be renewed or settled before expiry, or funds require a costly unilateral exit. EBP makes this a managed schedule:

- **Expiry tracking.** Every inbound VTXO is recorded with its own expiry — board/refresh VTXOs live ~28 days, Lightning-received VTXOs 1–3 days, and spend VTXOs inherit expiry from their parent, so some may need refresh almost immediately. Refreshes in the last two days of a VTXO's lifetime are free on Second's server; the expiry queue and alerts are driven by real per-VTXO expiries with a configurable lead time.
- **Renewal policy.** Default: renew or settle any VTXO 7 days before expiry (configurable). At Level 0 the server renews automatically. At Level 1 the merchant receives approval requests at T-14, T-7, T-3, and T-1 days via push and email; the flow is designed so a merchant who acts on any single alert is safe.
- **Sweep triggers.** Sweeps use Ark sends (same-ASP destinations) or cooperative offboarding to the merchant's on-chain address — never emergency exits, which require ≥15,000 confirmed sats per VTXO in CPFP fees and take 1–2 hours. The till keeps an on-chain fee reserve so an emergency exit is always fundable. Sweep when: the balance exceeds the hot cap; the oldest VTXO passes an age threshold; expiry approaches; or the merchant taps "Sweep now."
- **ASP failure.** If the ASP stalls cooperative operations, the dashboard says so in plain language, links the unilateral-exit runbook with current cost estimates, and keeps monitoring. ASP reachability and health history are logged alongside the settlement log.
- **Holding limits.** Docs recommend keeping hot float small and sweeping to on-chain for treasury; the defaults encode this.

## Payments: rails, pricing, states, checkout

**Rails.**

- **Ark (primary).** Instant, low-fee, self-custodial. "Paid" = the invoice's VTXO is received in the wallet. Ark's instant settlement carries ASP-related assumptions; the docs state them in plain language. Merchants can set a finality policy above a configurable amount: default is "final when the payment's VTXO is refreshed in a round and the round transaction confirms" (eliminates the sender-collusion trust window per the Ark docs); the conservative option offboards to on-chain confirmations.
- **On-chain (fallback, v1.0).** "Paid" = N confirmations (default 1, configurable); 0-conf available with an explicit warning.
- **Lightning receive (v1.0, core).** Via the ASP's built-in Lightning gateway: the Ark server operates its own Lightning node; the wallet generates the preimage, the server funds an HTLC from its VTXO pool, and payment reveals a spend VTXO — no swap provider at all. Lightning is included unconditionally; ASP fees must be displayed before checkout (the fee display is a Phase 1 close-out item) and the gateway trust model (server-pool HTLC with ephemeral key) is documented in-product. The trust window is about attestation, not availability: the received VTXO is spendable immediately — observed live on Second's signet ASP with barkd 0.8.0 (5,000-sat invoice, spendable on the first poll, `claimable_lightning_receive_sat` never above zero) — but it is ASP-attested until a round refresh boards it; that refresh, not claiming, ends the trust window. A mild, documented trust window in one rail does not justify removing the rail customers actually have. Lightning-received VTXOs still expire in 1–3 days regardless of instant claimability, so they are refreshed or swept promptly by policy.

**Fiat pricing & rate lock.** Merchants price in their own currency. At invoice creation EBP locks a BTC rate from the configured source (as built, the default source is `static` — deterministic offline rates for test mode; `live` switches to the median of major public APIs. The source is recorded per invoice) for the invoice lifetime (default 20 minutes). Amounts round to the sat. There is no live re-pricing: the customer sees a fixed sat price with a countdown; if it expires, they start a fresh invoice at the current rate. Fiat value, rate, source, and timestamp at payment time are stored for accounting.

**Invoice state machine.**

```
pending → paid | underpaid | overpaid | expired | cancelled
underpaid → expired (customer starts a new invoice at the current rate)
```

- **Underpaid:** the invoice expires when the countdown ends; the customer starts a fresh invoice if they still want the item (no partial-payment top-up UX in v1.0).
- **Overpaid:** marked clearly and treated as paid; the overage handling is the merchant's manual policy (v1.0 has no refund tooling).
- **Expired:** the rate lock lapses after 20 minutes (default); the invoice can be regenerated with one tap.
- **Refunds are out of scope for v1.0** (moved to v1.x). Bitcoin payments are final; checkout states this plainly and links the merchant's own policy page. The merchant handles any refunds manually from their own wallet — which is honest: the money is already theirs. v1.x may add assisted refund flows (Ark address / Lightning reverse swap) once core flows are proven.

**Checkout UX (customer).** Mobile-first page: fiat amount and sats, a combined BIP321 QR by default (`bitcoin:?amount=…&lightning=…&ark=…`) with per-rail tabs as the degraded mode, copyable address, live countdown, live status polled ~1/s (server-sent events remain a v1.x option), a plain "paid — Bitcoin payments are final; refunds follow this store's policy" line linking the merchant's policy, and a human-readable expired state. No jargon, no account, no PII required. Theming at v1.0: merchant logo + accent color.

**Merchant UX.** In Phase 1, invoices are created by the store through the API; the dashboard is a static info shell and cannot create invoices. Payment arrives → the invoice state changes → settlement happens per policy. Push/email notifications, the in-dashboard till view, and the settlement log are v1.0 targets. Every screen answers three questions: what happened, what happens next, what to do if something looks wrong.

## API sketch

Versioned REST, JSON. Store calls authenticate with per-store API keys (`Authorization: Bearer …`, hashed at rest, rotatable). Phase 1 implements only the routes listed in `README.md` (invoice create/get/list/cancel, health, checkout, dashboard); the additional routes below are v1.0 targets, not current endpoints. The dashboard uses an admin session (passkey or TOTP) in the v1.0 target.

```
POST   /api/v1/invoices
  { "order_id": "wc-1234",
    "fiat": { "currency": "EUR", "amount": "42.00" },     # or "sats": 61000
    "expiry_minutes": 20, "metadata": { ... } }
  → 201 { "id": "inv_…", "status": "pending", "amount_sats": 61000,
          "rate": "68852.41", "rate_source": "median(kraken,bitstamp)",
          "checkout_url": "https://pay.example.com/i/inv_…",
          "ark_address": "ark1…", "onchain_address": "bc1q…",
          "expires_at": "…" }
  # Idempotent per order_id: if the store already has an OPEN (pending,
  # unexpired) invoice for the same order_id AND the same amount (fiat
  # amount+currency, or sats), that invoice is returned with 200 instead.
  # A different amount creates a new invoice. (beta hardening)

GET    /api/v1/invoices/{id}            # source of truth for stores
GET    /api/v1/invoices?status=&since=
POST   /api/v1/invoices/{id}/cancel
GET    /api/v1/unmatched-payments       # late payments to expired/closed invoices (store-scoped)
POST   /api/v1/unmatched-payments/{id}/resolve   # mark handled (refund / manual fulfilment)
GET    /api/v1/rates?currency=EUR
GET    /api/v1/settlements              # expiry queue + history
POST   /api/v1/settlements/sweep-now
GET    /api/v1/health                   # server, wallet, ASP reachability, watcher_last_tick_age_secs
GET    /api/v1/exports/accounting.csv?from=&to=
POST   /api/v1/stores/rotate-key
```

## Webhooks & store-side verification

- Events: `invoice.paid`, `invoice.underpaid`, `invoice.overpaid`, `invoice.expired`, `settlement.completed`, `health.alert`.
- Every delivery carries `X-EBP-Event`, `X-EBP-Idempotency-Key`, and `X-EBP-Signature: t=<unix>, v1=<HMAC_SHA256(store_webhook_secret, t + "." + body)>`.
- Stores must: verify the HMAC with their per-store secret; reject timestamps older than 5 minutes; dedupe on the idempotency key; and treat `GET /api/v1/invoices/{id}` as the source of truth before fulfilling — this defeats forged-webhook fraud even if a secret leaks.
- Phase 1 performs a single delivery attempt. Retries with backoff (1m / 5m / 30m / 2h / 12h), a delivery log in the dashboard, one-tap re-send, and secret rotation without downtime (two active secrets during rotation) are Phase 2 work.
- Webhook payloads contain no customer PII by default.

## Data model (outline)

- `stores(id, name, api_key_hash, webhook_url, webhook_secret, policy_json, created_at)` — the data model is store-scoped from day one, even while the v1.0 UI manages a single store. API keys are hashed at rest (the server only verifies them). The webhook secret must be recoverable in plaintext to sign outbound deliveries, so it cannot be hashed; the v1.0 target is encryption at rest via `key_encryption_key` (Phase 1: plaintext in the access-restricted SQLite datadir — documented, not hidden).
- `invoices(id, store_id, order_id, status, amount_sats, fiat_currency, fiat_amount, rate, rate_source, rate_locked_at, ark_address, onchain_address, expires_at, paid_at, paid_sats, confs_required, metadata_json)`
- `payments(id, invoice_id, rail, amount_sats, vtxo_id | txid, detected_at, confirmed_at)`
- `settlements(id, kind[sweep|renew|exit], amount_sats, destination, status[proposed|awaiting_approval|broadcast|confirmed|failed], approved_by, created_at, confirmed_at)`
- `webhook_deliveries(id, invoice_id, event, payload_json, attempts, last_status, next_retry_at)`
- `vtxo_expiry(id, vtxo_ref, amount_sats, expiry_at, renew_by, status)` — feeds the expiry queue
- `audit_log(id, actor, action, detail_json, created_at)` — every privileged action
- `settings(key, value)` — policies, caps, notification config, checkout policy text

## Security & threat model

A living document, updated at every phase gate. Headline entries:

- **Payment redirection (the realistic attack).** A compromised server can show attacker addresses at checkout. Mitigations: the merchant's phone displays the wallet's own receive addresses for comparison; a guided "verify your checkout" recipe (pay yourself 100 sats) runs at setup and after every update; instant payment notifications show amount and destination; the dashboard has an address page.
- **Key theft.** The server wallet holds a capped float; a compromise can steal at most the till, never swept funds. As built (Phase 1): wallet keys live in barkd's access-restricted datadir; EBP-side at-rest encryption of webhook secrets is not yet implemented (`key_encryption_key` is a placeholder — tracked); the barkd token is env-only and never logged; the mnemonic is shown once via a one-time expose-mnemonic ceremony (barkd is restarted WITH --expose-mnemonic for the ceremony, the merchant taps "Show my recovery words", and it is restarted without the flag afterward), never stored, logged, or re-readable after. "Encrypted at rest" (keys, secrets, backups) is a v1.0 target, not a current property — the current control is datadir restriction. No key material in logs, ever — enforced and re-checked at each gate.
- **Forged webhooks / replay.** HMAC + timestamp window + idempotency + store-side verification (above). As built (Phase 1): a single delivery attempt, no retry/backoff queue yet — the store-side source-of-truth check is what makes that safe; retries, the delivery log, and secret rotation land in Phase 2 as specified.
- **Data breach.** No customer PII required; minimal logs; encrypted backups; retention controls (see Privacy).
- **DoS.** Rate limits on public endpoints; static checkout pages; store-side verify keeps orders safe while the server is down (orders queue as unpaid).
- **ASP misbehavior.** Non-custodial ≠ trustless: the ASP can stall but not steal. Monitoring, health history, exit runbook, own-ASP guide.
- **Swap counterparty (v1.x Lightning).** Documented trust model, fee display, and stuck-swap runbook are prerequisites for that rail shipping.
- **AI agent misuse.** Recipes never contain or request secrets; agents never receive keys, seeds, or admin credentials; destructive steps require human confirmation in the product UI.
- **First-boot window.** The setup wizard is unauthenticated until `setup_complete` (the BTCPay first-run model). On a deployment with a public IP, anyone can race the merchant to wallet creation and receive the mnemonic; wizard pages also carry the mnemonic and barkd token over plain HTTP until TLS is configured. Mitigations: complete setup immediately after first boot; keep the server firewalled or use an encrypted path (SSH tunnel/Tor) for remote first run; treat a wallet that exists before you ran the wizard as compromised (reset the datadir).
- **Webhook URL as an SSRF surface.** The store's webhook URL is merchant-controlled and the server POSTs to it. Scope: authenticated merchants only, POST-only, no upstream response bodies reflected to the caller; barkd is unreachable without its bearer token, which webhook requests do not carry. Hardening (Phase 1 close-out): reject loopback/link-local/metadata targets by default.
- **Sweep finality.** Ark sweeps to the merchant's wallet are out-of-round (arkoor) sends: the destination VTXO is spendable immediately but is ASP-attested until the merchant's own wallet boards it in a round. EBP's invoice finality policy governs inbound customer payments; sweeps deliver out-of-round VTXOs by design, and the docs say so at the sweep setting.
- Every gate's security review explicitly covers: payment redirection, webhook secret handling, custody-level claims, and (when present) the swap path.

## Privacy, accounting, compliance

- **Data minimization.** Invoices need only an amount and an opaque order reference. No customer accounts, no tracking pixels, no third-party calls from checkout.
- **GDPR baseline.** Configurable log retention (defaults: access logs 7 days, no IPs in application logs); a per-invoice erasure endpoint that preserves accounting-required aggregates; self-hosting means the merchant is the data controller — docs say so and give them the words for their own privacy policy.
- **Accounting & tax.** Every paid invoice records fiat value, rate, rate source, and timestamp at payment time. CSV export: date, order ref, sats, fiat value at receipt, rail, fees, settlement reference. This is the minimum real merchants need; additional formats arrive with community demand.

## Store connectors

- **WooCommerce (v1.0).** Thin plugin: creates invoices on order, verifies webhooks (HMAC + source-of-truth check), maps invoice states to order statuses, shows plain-language status. Settings are three fields: server URL, API key, webhook secret.
- **Shopify (v1.x).** App review, OAuth, and compliance are a project of their own; scheduled after v1.0.
- **Connector interface (v1.x).** A documented contract so Medusa, Saleor, and custom stores can integrate; community plugin program.

## Deployment

One-click is a requirement, and "click" includes *reachable*:

- **Docker Compose on a VPS (primary and, for now, only).** The supported deploy is the Docker Compose stack on a VPS with a public IP; TLS terminates at the merchant's own reverse proxy. Automated one-click TLS is a v1.0 gate. Multi-arch images (x86_64, arm64), persistent volume, healthchecks, automatic SQLite migrations, guided backups, and a tested upgrade path are release requirements.
- **VPS-first hosting.** A small business checkout must be reachable by customers over clearnet HTTPS; a $5 VPS with DNS + TLS is the deliberate answer. Home/Tor-only hosting cannot take customer payments and is out of scope.
- **No managed-VPS image in v1.0.** Marketplace-style images are not a supported path.
- **Start9 (v1.x); bare binary (advanced).** The binary is for advanced users and documented as such.
- Every release: all one-click paths are installed from scratch *and* upgraded from the previous release in CI before the release is tagged.

## iPhone experience & notifications

- **Dashboard = PWA.** Install-to-homescreen is a guided setup step. Every admin action is phone-completable.
- **Notifications.** Web Push (iOS 16.4+) is the primary channel; email (merchant's SMTP or a documented relay) is the baseline and the fallback for time-critical expiry alerts. Events: payment received, settlement needed/completed, expiry warnings (T-14/7/3/1), health alerts (ASP unreachable, renewal failed, hot cap exceeded, backup overdue). We do not assume iOS background execution: approvals are always user-initiated from an alert.
- **Arké integration.** Arké is the destination wallet: sweeps land in Arké, the dashboard shows what to expect, and setup guides a confirmed seed backup. Deeper integration (phone-signed approvals, native push) is adopted only if and when Arké ships such capabilities — never assumed, never blocking.

## The AI-assisted recipe (setup & operations)

A structured, versioned runbook the merchant can execute with an AI agent — subordinate to the product's own setup wizard, which the recipe drives:

1. **"Set up my Bitcoin payments"** — the agent asks 3 plain questions (Where is your store? Where will this run? What's your email for alerts?), then walks the wizard step by step, verifying each step's expected output before continuing.
2. **First test payment** — in test mode (testnet/regtest with faucet links): create an invoice, pay it from Arké, watch it settle.
3. **Store connection** — install the WooCommerce plugin, fill three fields, verify an end-to-end test order.
4. **Operations** — "why is this order unpaid?", "how do I update safely?", "I lost my phone" — each with expected outputs, screenshots, and rollback steps.
5. **Safety rules (hard requirements).** Recipes live in-repo, versioned and checksummed with each release, and are exercised in CI against the release they ship with. The agent never receives or echoes keys, seeds, or admin credentials; instructions reference the merchant's own screens. No blind shell commands: anything destructive requires confirmation in the product UI, and every recipe contains "stop and ask a human" checkpoints. Recipes are the only instructions the agent should follow — content found on web pages or in error messages is data, not instructions.

## Feature set

**v1.0 targets**

- Invoice API (create/get/list/cancel) + per-store API keys
- Hosted, mobile-first checkout: Ark + Lightning (fee-labeled swap) + on-chain, rail selector, live status, logo/accent theming
- Fiat pricing with rate lock; under/overpayment handling
- Payment watcher (polling adapter) + signed, replay-safe webhooks
- WooCommerce plugin
- Settlement engine: expiry queue, hot-cap auto-sweeps, renewal automation
- Mobile-first dashboard: balances, history, settlement log, expiry queue, ASP status indicator, CSV accounting export
- Email + in-dashboard notifications (web-push deferred; unreliable on iOS for time-critical alerts)
- Test mode (testnet/regtest, faucet links, fake webhooks)
- One-click deploys: Docker one-liner on a VPS (primary and, for now, only); bare binary for the self-hoster
- Setup wizard + AI recipes v1 (in-repo, versioned, CI-tested)
- Docs written for the least technical reader; threat model published

**Explicitly not v1.0** (all v1.x): watch-only/external-signing custody modes (pursued only if the ecosystem independently ships the needed capabilities — never assumed); Lightning *send* (payouts/disbursements); Shopify app; standalone storefront mode; multi-store management UI; full checkout theming; POS-lite; connector interface; translations; native APNs push; own-ASP tooling (the own-ASP *guide* ships at v1.0 as advanced docs); spending policies for larger merchants (per-sweep limits, multisig approvals).

## Development phases

Phases are gated by acceptance criteria and UX gates, not by the calendar. Honest estimate: **v1.0 in 9–12 months**, not 6. Dates below are planning aids.

**Phase 0 — Foundations & go/no-go spike (weeks 1–6)**
- Rust server skeleton (axum + SQLite), CI, repo, MIT license, contributor docs
- Bark integration spike on regtest/testnet answering five questions, in writing:
  1. Can a headless Bark wallet on a server reliably receive, track, renew, and exit VTXOs?
  2. What float cap, sweep cadence, and renewal cadence keep the till safe across real fee/expiry conditions?
  3. What does the Boltz VHTLC swap path look like end-to-end from bark-wallet (fee profile, stuck states, failure recovery)?
  4. What does Arké support today for receive, and what address format should sweeps target?
  5. What observation does the ASP support (polling endpoints, any subscription)?
- One real testnet sweep; a written record confirming the Level 0 till design against observed behavior
- UX prototype: checkout, dashboard, installer, recipe format (Figma → Tailwind components)
- Threat model v0
- **Acceptance:** regtest demo of receive → renew → exit; go/no-go document published; a non-technical tester completes the clickable prototype's core flow unassisted.

**Phase 1 — Core payments MVP (months 1–3)**
- Invoice API, API keys, hosted checkout (Ark + on-chain), FX rate lock
- Watcher (polling) + signed webhooks; basic mobile dashboard
- Test mode; expiry tracking + alerts; Docker one-liner; VPS image started
- Setup wizard v1 + AI recipe v1 (setup → first test payment)
- **Acceptance:** end-to-end testnet payment demo; webhook verification test suite passing; **UX gate: a non-technical tester goes zero → live testnet invoice using only the recipe and an iPhone.**

**Phase 2 — First store + settlement (months 3–6)**
- WooCommerce plugin; Lightning receive via the ASP's Lightning gateway (fee display + trust-model doc are release prerequisites); settlement engine (auto-sweep, expiry renewals, sweep-now)
- Managed-VPS image; accounting CSV; backup/restore
- AI recipes: store connection, troubleshooting, lost phone
- Security review #2 (redirection, webhooks, custody claims)
- **Acceptance:** **UX gate: a non-technical merchant connects a real WooCommerce store unassisted, takes a test payment, and confirms a sweep arrived in their wallet — all from an iPhone.** One pilot merchant transacting on mainnet with small amounts.

**Phase 3 — v1.0 (months 6–12)**
- Hardening: multi-arch releases, upgrade path tested, docs pass, recorded usability sessions (clean markup, keyboard navigation, and contrast by default; a formal WCAG AA audit is v1.x)
- Own-ASP advanced guide (docs); typical-cost tables in docs
- 2+ real stores transacting; every one-click deploy path tested from scratch and by upgrade
- **Acceptance (v1.0):** every v1.0 feature above shipped; threat model current; CHANGELOG honest; docs pass the non-technical test; at least one genuinely non-technical tester signs off each gate.

**Phase 4 — Reach (v1.x, ongoing)**
- Lightning send/payouts; watch-only custody only if the ecosystem independently enables it
- Shopify app; POS-lite PWA for in-person; standalone storefront mode; multi-store UI; full theming
- Connector interface + community plugin program; translations; native push via Arké when available; spending policies for larger merchants

**Gates between every phase:** security review (the custody claims must hold — threat-model doc updated), honest CHANGELOG, and the phase's UX gate signed off by at least one genuinely non-technical tester before anything ships.

## Open questions (tracked publicly)

1. Bark watch-only/descriptor mode and external-signer interface — not assumed and not a dependency; EBP designs around current Bark/Arké capabilities.
2. Ark signing-request format + transport (deep link / QR) and Arké's roadmap for it.
3. A standard multi-rail payment URI (Ark + on-chain ± Lightning) — adopt, don't invent, if the ecosystem converges.
4. Level 2 restricted signer: what Ark script/policy can enforce today.
5. ASP subscription/push APIs vs polling; ASP fee models and how they surface to merchants.
6. Reliability of iOS web push for time-critical expiry alerts (mitigations: email fallback, generous windows, Level 0 auto-renewal).

## Project shape

- **Language:** Rust (core + server); Phase 1 dashboard/checkout are static HTML/JS with no build step. TypeScript is only a future option if a frontend build step is added. Thin connectors live in each store's native ecosystem.
- **No upstream assumptions:** EBP works with Bark and Arké as they exist today. Coordination with their maintainers is welcome, but the roadmap never depends on another project shipping anything.
- **License:** MIT
- **Governance:** meritocratic open source; Second/Bark and Bitcoin Design Community collaboration encouraged — Easy Bitcoin Payments aims to be the reference commerce layer of the Bark ecosystem, not a competitor to it.
- **Funding:** donations, grants (Spiral/HRF/OpenSats); no VC. Free forever is the product.

## Why this exists

BTCPay Server proved merchants want self-custodial, feeless Bitcoin payments — and its compromises (legacy integrations, a large C# codebase, sysadmin-grade operations) define what a second-generation implementation must fix. Ark removes Lightning's merchant pain (no channels, no inbound liquidity, passive receive), and Bark is the production-grade Rust implementation. The gap is not the rails — it's the commerce layer, and it must be built honestly: passive receive is real today, watch-only sovereignty is not assumed, and EBP designs around current Bark/Arké capabilities rather than waiting on upstream work. Easy Bitcoin Payments is that layer: free, open, sovereign, modern, and as easy as installing an app on the phone already in your pocket.