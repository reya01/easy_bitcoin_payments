=== VERDICT ===
Promising mission, but v1.0 as specified is not feasible in 6 months. The core “xpub-only server + iPhone-signed Ark sweeps/refunds” model is unproven on Bark/Arke today and is the project’s biggest technical risk. Narrow to a Docker/WooCommerce/Ark-receive MVP first.

=== CRITICAL ISSUES ===
- Bark watch-only/xpub-only operation is not established. Bark is a wallet; it may require seed/private keys to derive Ark addresses, track VTXOs, refresh, and interact with the ASP. Do not claim “server holds only public keys” until Bark exposes a supported watch-only/external-signer mode.
- iPhone-signed Ark sweeps/refunds are not a standard PSBT flow. Ark VTXO spending involves ASP coordination, expiry/refresh semantics, and Bark-specific signing. Arke may not expose a server-proposes/phone-approves API. This must be proven in Phase 0, not assumed.
- VTXO expiry/refresh requires signing before expiry. If the phone is offline, the server cannot refresh or sweep. “Passive receive” may be true; “passive settlement” is not. Funds can be at risk or stuck without timely phone approval.
- “No middleman” is false for the Lightning rail. VHTLC/Boltz swaps introduce a swap counterparty, liquidity, fees, and failure modes. Ark itself depends on an ASP for liveness/coordination; non-custodial ≠ trustless.
- Payment watcher “subscribes to the Bark ASP” assumes an ASP subscription/webhook API that may not exist. Likely need polling Bark wallet state or building ASP-side indexing. Verify before promising real-time webhooks.
- “One QR, all rails” lacks a standard Ark payment URI and unified fallback semantics. BIP21 + Lightning + Ark + on-chain in one scannable instruction is a UX/protocol design project, not a checkbox.
- Server compromise can still cause payment fraud: attacker can forge “paid” webhooks to the store, alter invoices, leak order data, or DoS. Non-custodial funds do not mean non-custodial payment state. Webhook HMAC, timestamps, replay protection, secret rotation, and store-side verification are mandatory.
- Refund trust is unresolved: merchant phone approval prevents server theft, but customers still trust the merchant to refund. Define refund policy, expiry, partial refunds, and dispute handling.
- Missing merchant-critical features: chargeback/irreversibility messaging, fiat price lock/volatility handling, under/overpayment rules, accounting/tax CSV with fiat values, GDPR/data-retention policy, and support runbooks for stuck swaps/expired VTXOs.

=== SHOULD FIX ===
- Reframe claims: “non-custodial, ASP-dependent, phone-required for settlement/refresh” rather than “no middleman, no custodian.”
- Add a Phase 0 go/no-go spike: Bark watch-only, Arke external signing, VTXO refresh, ASP observation, and one real testnet sweep. If any fail, redesign before building commerce features.
- Define webhook security precisely: HMAC-SHA256, timestamp window, idempotency key, replay cache, TLS pinning/verification, per-store secrets, and store-side “verify payment” API.
- Add invoice state machine: pending/paid/underpaid/overpaid/expired/refunded, with explicit merchant/customer messaging.
- Add fiat pricing: rate source, quote lock duration, invoice expiry, rounding, and accounting export at payment-time rate.
- Add privacy/GDPR baseline: data minimization, log retention, no customer PII in webhooks by default, erasure workflow, self-hosting guidance.
- Add backup/restore and seed/phone-loss recovery flows. A merchant losing the iPhone must not lose funds or settlement ability.
- Treat Shopify as post-v1.0. Shopify app review/OAuth/billing/compliance is a large separate project.
- Treat standalone storefront and multi-store as post-v1.0 unless a pilot merchant requires them.
- One-click deploy needs more than a Rust binary: multi-arch containers (arm64 for Umbrel), persistent volumes, healthchecks, migrations, backups, update path, Tor/SSL guidance, and app-store packaging/review.
- AI recipes must be versioned, tested against releases, checksummed, and never require blind sudo/shell commands. Prefer UI-guided steps; include rollback and “stop, ask a human” checkpoints.

=== NICE TO HAVE ===
- Push notifications via APNs require Arke app changes and a notification backend; do not assume iOS background execution enables automatic approvals.
- Own-ASP guide is valuable but operationally heavy; keep as advanced docs, not v1.0 dependency.
- Accessibility WCAG AA is good but schedule after core payment flows are stable.
- Translations and connector interface can wait for community demand.
- Consider a “merchant test mode” with regtest/testnet invoices and fake webhooks before mainnet.

=== WHAT'S STRONG ===
- Clear mission and strong UX principle: non-technical tester gates are excellent.
- Sensible stack choice: Rust, SQLite, single binary, Docker-first is appropriate for self-hosting.
- Thin store connectors calling the merchant’s own server is the right privacy architecture.
- Explicit threat-model/security-review gates and honest CHANGELOG intent are good.
- Focusing on Ark’s passive receive as the first rail is the right wedge if Bark supports it.
- AI recipes as plain-text, in-repo, versioned runbooks with human fallback is the right shape—if kept subordinate to tested UI flows.