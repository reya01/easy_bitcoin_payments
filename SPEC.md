# Easy Bitcoin Payments

*A free & open-source payments layer for online stores, built on Ark and Bark. Run a sovereign, Bitcoin-first business from your own server — manageable entirely from an iPhone.*

---

## Mission

Every online business should be able to accept Bitcoin **directly, for free, with no custodian and no middleman** — and running the software should be as easy as installing an app. Easy Bitcoin Payments is free, open-source software any merchant can self-host, connecting their store to Bitcoin's newest payment rail (Ark) plus Lightning and on-chain, while their keys never leave their hands — and their entire operation fits in their pocket.

BTCPay Server's mission, rebuilt on a modern stack for the Ark era: **one-click deploys, iPhone-first management, and AI-assisted setup.**

## Principles

1. **Sovereign by default.** Funds settle to the merchant's own keys. The software holds only public keys; it cannot spend merchant funds even if fully compromised. No account can be frozen, no gateway can deplatform you, no processor can leak your customer list.
2. **Bitcoin only, merchant's choice beyond.** The reference implementation accepts Bitcoin exclusively — that's the point. Bitcoin-preferred setups (a discount for paying in Bitcoin, Bitcoin-first checkout with optional fiat fallback) are supported as opt-in configuration, never defaults.
3. **Free forever, open forever.** MIT-licensed. No hosted middleman tier, no transaction fees to anyone. The protocol dependency (the Bark ASP) is the only service relationship, and merchants can run their own.
4. **Easy is the product.** As simple to run as a consumer app. One-click deploys are always the goal — never an afterthought. A merchant who has never opened a terminal should succeed.
5. **Modern stack, boring operations.** Rust core, SQLite by default, single binary, well-documented. No C# monolith, no obsolete-protocol baggage, no node-management burden.
6. **Non-technical by design.** Every flow is designed for someone who has never touched a terminal: plain-English settings, sensible defaults, human-readable errors that say what happened and what to do next, and jargon translated on sight. Where Bitcoin concepts are unavoidable (keys, sweeps, expiry), the UI teaches with plain words and never requires understanding internals to succeed. **No feature ships until a non-technical tester completes its flow unassisted.**
7. **Manage everything from an iPhone.** The full business loop — wallet, payments, settlements, refunds, store health — must work end-to-end from a phone. [Arké](https://arke.cash/) (the design-led, open-source iOS Bark wallet by Christoph Ono / Bitcoin Design Community, built in SwiftUI over Bark's FFI bindings) is the reference wallet: payments land directly in the merchant's Arké wallet, and settlement, refund, and monitoring actions route through it. No desktop required, ever.
8. **AI-assisted, not AI-dependent.** Setup and operations include a guided, AI-assisted recipe: an agent (local or the merchant's preferred assistant, e.g., Claude/ChatGPT via shareable runbooks) walks non-technical users through install, store connection, first test payment, and troubleshooting. The AI layer reads the docs, not the user's keys. Every recipe has a plain-text fallback so the software remains fully usable without AI.

## What it is

A single self-hosted service ("the payments server") that sits between an online store and Bitcoin:

```
Store (WooCommerce, Shopify, custom, or standalone storefront)
   │  order → HTTP POST /invoices
   ▼
Merchant's own Easy Bitcoin Payments server (self-hosted, one binary)
   ├── Invoice API — payment requests paying to the MERCHANT's
   │   own keys (xpub stored locally; private keys never on server)
   ├── Checkout page — static, themable; one QR covering Ark +
   │   Lightning (Boltz VHTLC swap) + on-chain fallbacks
   ├── Payment watcher — subscribes to the Bark ASP; fires signed
   │   webhooks when paid; idempotent, replay-safe
   ├── Settlement engine — scheduled or expiry-horizon sweeps to
   │   the merchant's on-chain address, signed via the merchant's
   │   iPhone wallet (Arké) — approve on the phone, never type keys
   ├── Refunds — back to customer Ark address / Lightning reverse
   │   swap / on-chain, approved on the phone
   └── Merchant dashboard — mobile-first PWA + iPhone-native flows
   ▼
Bark ASP (Second's public server, or merchant's own barkd)
   └── passive VTXO receive → merchant keys (Arké wallet)
```

## Key design decisions

- **Receive without signing.** Ark's passive receive means accepting payment requires no merchant action and no hot keys. Sweeps/spends are signed on the merchant's iPhone (Arké) — the server proposes, the phone approves.
- **iPhone is the admin console.** The dashboard is mobile-first; every operational action (approve settlement, issue refund, check store health, read alerts) is completable on a phone screen. Push notifications for payments and alerts. If a flow can't be completed on a phone, it isn't done.
- **ASP-agnostic.** Works with Second's public ASP out of the box; supports any Bark-compatible ASP including the merchant's own `barkd`.
- **One QR, all rails.** Ark preferred (instant, cheap, self-custodial), Lightning and on-chain as automatic fallbacks in one payment instruction.
- **Store connectors are thin clients.** WooCommerce plugin and Shopify app call the merchant's own server. No third-party data flows.
- **Standalone storefront mode.** A minimal built-in catalog + cart + checkout for Bitcoin-only businesses that want no platform at all — their site, their server, their coins.
- **One-click deploys, as a spec requirement.** Target platforms, in order: Umbrel app (one click, fits the Bark/BTC ecosystem), Docker/Podman one-liner, Start9, managed-VPS image (Hetzner marketplace-style), and a plain binary. A new release is not "done" until its one-click paths are tested.

## The AI-assisted recipe (setup & operations)

A structured, versioned runbook the merchant can execute with an AI agent:

1. "Set up my Bitcoin payments" — the agent asks 3 plain questions (Where is your store? Where will this run? What's your email for alerts?), then issues exact commands/links step by step, verifying each step's expected output before continuing.
2. First test payment: agent guides creating a test invoice and paying it from Arké.
3. Store connection: agent walks the WooCommerce/Shopify connector, then verifies an end-to-end test order.
4. Ongoing operations: recipes for "why is this order unpaid?", "how do I refund?", "how do I update safely?", each with expected outputs and screenshots.
5. The recipes live in-repo, versioned with releases, and are plain-text so any AI assistant (or a human friend) can execute them. The agent never receives keys, seeds, or admin credentials — instructions reference the merchant's own screens.

## Feature set (v1.0 targets)

- Invoice API (create/get/list/cancel) + API keys per store
- Hosted, themable checkout pages (mobile-first)
- Webhooks (paid/expired/refunded) with signature verification
- WooCommerce plugin; Shopify app
- Ark + Lightning + on-chain in one checkout
- iPhone-signed settlement sweeps and refunds (Arké integration)
- Push notifications for payments, settlements, and health alerts
- Mobile-first dashboard: balances, history, settlement log, CSV export
- Multi-store support on one server
- One-click deploys: Umbrel app, Docker one-liner, managed-VPS image, bare binary
- AI-assisted setup & troubleshooting recipes (in-repo, versioned)
- Documentation written for the least technical reader

## Development phases

**Phase 0 — Foundations (weeks 1–4)**
- Rust server skeleton (axum + SQLite), CI, repo, MIT license, contributor docs
- `bark-wallet` integration spike on testnet/regtest: receive VTXO, observe expiry semantics; document the Arké signing-flow requirements
- UX prototype: checkout page, dashboard, installer, and the AI-recipe format (Figma → Tailwind components)
- Deliverable: regtest VTXO demo + clickable UX prototype + draft AI recipe

**Phase 1 — Core payments (MVP, months 1–2)**
- Invoice API, API keys, hosted checkout page (Ark receive first)
- Payment watcher + webhooks; basic dashboard (mobile-first)
- One-click deploy #1: Docker one-liner; Umbrel app package started
- AI recipe v1 for setup-to-first-test-payment
- **UX gate:** a non-technical tester goes zero → live testnet invoice using only the recipe and an iPhone
- Deliverable: end-to-end testnet payment demo

**Phase 2 — First store + Lightning (months 2–4)**
- WooCommerce plugin; Lightning receive via VHTLC swap; unified QR
- Settlement engine with iPhone (Arké) approval flow
- Refund flow; Umbrel app published; managed-VPS image
- AI recipes for store connection + troubleshooting
- **UX gate:** non-technical merchant connects a real WooCommerce store unassisted, takes a test payment, issues a refund — all from an iPhone
- Deliverable: one pilot merchant transacting on mainnet (small amounts)

**Phase 3 — Everyday usability (months 4–6)**
- Full Arké integration: settlement approvals, refund approvals, health alerts as push notifications
- Multi-store, CSV exports, themable checkout
- Standalone storefront mode (catalog + cart + checkout)
- Accessibility pass (WCAG AA); docs pass; recorded usability sessions
- Deliverable: **v1.0** — feature-complete, 2+ real stores, every one-click deploy path tested, docs pass the non-technical test

**Phase 4 — Reach (v1.x, ongoing)**
- Shopify app; POS-lite PWA for in-person
- Own-ASP guide and tooling (run `barkd` alongside)
- Connector interface for other platforms (Medusa, Saleor, custom); community plugin program
- Translations

**Gates between every phase:** security review (the non-custodial claims must hold — threat-model doc updated), honest CHANGELOG, and the phase's UX gate signed off by at least one genuinely non-technical tester before anything ships.

## Project shape

- **Language:** Rust (core + server), TypeScript for dashboard/checkout frontends, thin connectors in each store's native ecosystem
- **License:** MIT
- **Governance:** meritocratic open source; Second/Bark and Bitcoin Design Community collaboration encouraged — Easy Bitcoin Payments aims to be the reference commerce layer of the Bark ecosystem, not a competitor to it
- **Funding:** donations, grants (Spiral/HRF/OpenSats); no VC. Free forever is the product.

## Why this exists

BTCPay Server proved merchants want self-custodial, feeless Bitcoin payments — and its compromises (legacy integrations, a large C# codebase, sysadmin-grade operations) define what a second-generation implementation must fix. Ark removes Lightning's merchant pain (no channels, no inbound liquidity, passive receive), and Bark is the production-grade Rust implementation. The gap in 2026 is not the rails — it's the commerce layer. Easy Bitcoin Payments is that layer: free, open, sovereign, modern, and as easy as installing an app on the phone already in your pocket.
