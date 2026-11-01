=== VERDICT ===
Worth building: a BTCPay-grade commerce layer for Ark is a real gap, and the core thesis is sound — passive receive genuinely removes Lightning's channel/inbound-liquidity pain for merchants. But the spec systematically overstates what Bark/Arké can do today: the iPhone-signing flow, watch-only xpub mode, and multi-rail QR are things you must invent, not integrate. The non-custodial claim has real holes, and v1.0 in 6 months is 2–3x optimistic. Greenlight Phase 0 as a genuine kill-gate spike; re-scope v1.0 to Ark + on-chain + WooCommerce; treat 12–18 months as the honest v1.0 horizon.

=== CRITICAL ISSUES ===
1. The Arké signing flow does not exist. "Server proposes, phone approves" requires a PSBT-equivalent for Ark virtual transactions, a transport (deep links), and coordination with ASP round timing. Arké is an early design prototype, not a production wallet, with no external-signing API. This is the project's hardest workstream and the spec presents it as integration. Budget it as protocol design plus upstream Arké work — or redesign.
2. Bark has no watch-only/xpub mode today. bark-wallet manages keys internally; deriving Ark addresses and observing/claiming VTXOs from an xpub-only server requires upstream Bark work, and VTXO scripts are not plain xpub derivations. Phase 0's spike must be allowed to fail and change the architecture (the fallback — full wallet on server — means keys on server).
3. VTXO expiry contradicts "no merchant action." Receiving is passive; holding is not. VTXOs must be renewed or exited before expiry or funds are at risk / forced into costly unilateral exits. With phone-only signing, a merchant who loses their phone or ignores alerts is in trouble. You need an explicit policy: e.g., a restricted server-side co-signer that can only renew/exit to the merchant's fixed on-chain address (compromise can force settlement home but cannot steal), plus sweep cadence and documented holding limits.
4. The Lightning path breaks either the principles or the checkout. Per-invoice VHTLC claims need signing — phone approval per customer payment is unusable. The workable shape is Boltz paying out to the merchant's Ark/on-chain address, but that is a third-party swap service with fees and a custody hop, contradicting "no middleman, no fees to anyone." Drop LN from v1.0 or amend the principles honestly.
5. "Cannot spend merchant funds even if fully compromised" is misleading. A compromised server generates the checkout QR — it can redirect new payments to attacker addresses. Payment redirection, not key theft, is the realistic compromise scenario. The threat model must say so and mitigate (wallet-side address verification, payment alerts).
6. Native iOS push requires Apple developer accounts, APNs, and a relay — a middleman and a dependency on the Arké team's infrastructure. Use PWA web push (iOS 16.4+) for v1, or make the relay an explicit documented dependency.
7. Six months to this v1.0 is not credible — it is BTCPay-scale scope, and BTCPay took years and many contributors. Cut from v1.0: standalone storefront, multi-store, themable checkout, LN swap. Keep: invoice API, Ark + on-chain checkout, watcher/webhooks, WooCommerce, Docker deploy, dashboard, expiry alerts, manual settlement via a documented Arké exit flow.

=== SHOULD FIX ===
- No FX rate story at all. Real merchants price in fiat: you need a rate source, per-invoice rate lock, expiry windows, and volatility messaging at checkout.
- Define "paid" per rail (VTXO round confirmation vs LN settled vs on-chain 0-conf/N-conf), plus under/overpayment handling. These are the everyday support tickets.
- "One QR, all rails" has no standard — no wallet parses a multi-rail URI today and Ark URI conventions are unsettled. Plan for rail-specific QRs or a selector; verify what Arké actually parses.
- ASP dependency honesty: the ASP can stall cooperative exits and refunds; unilateral exit costs fees and time. Document worst-case exit costs, ASP health monitoring, and pull the own-barkd guide earlier than v1.x for volume merchants.
- Accounting/tax: a CSV of BTC amounts is not enough. Record fiat value at time of receipt per invoice.
- One-click deploy reality: Umbrel packaging is easy (it's docker-compose); the hard unsolved part is reachability — TLS, domain, store→server callbacks from home/NAT setups. Ship VPS-first plus a documented tunnel option, or "one-click" becomes "one-click then a weekend."
- AI recipes: prefer driving a first-class setup wizard/API over shell commands for non-technical users; pin recipes to release versions; treat prompt-injection and hallucinated-command risk explicitly (expected-output verification is good — keep it); the agent must never output or echo secrets.
- GDPR/privacy: invoice PII, IP logging, retention policy, data-minimized checkout. EU merchants will ask.
- Customer-facing irreversibility messaging and refund-policy UX; young-protocol confusion (expiry, ASP downtime) lands on the merchant — give them the words and diagnostics.
- "Free forever, no fees" overstates: ASP fees, miner fees, and swap fees exist. Say "no software fees."
- Security review scope should explicitly cover payment redirection, webhook secret handling, and the swap path's trust model.

=== NICE TO HAVE ===
- Worst-case exit-cost calculator (unilateral exit at current fees with N outstanding VTXOs).
- Expiry calendar/queue in the dashboard: what must be signed by when.
- Address-verification flow in Arké (prove the checkout shows your address).
- Demo/testnet mode with faucet integration for the first-test-payment recipe.
- Spending-policy options for larger merchants (multisig approvals, per-sweep limits).
- ASP reachability/health history alongside the settlement log.

=== WHAT'S STRONG ===
- The gap is real and timing is right: no commerce layer exists for Ark, and passive receive is a genuine merchant win over Lightning.
- xpub-only server as the non-custodial north star is the correct construction, even where the marketing overstates it.
- UX gates with genuinely non-technical testers before each phase ships is rare discipline — keep it.
- Rust single binary + SQLite + bark-wallet crate is the right boring stack; ASP-agnostic with an own-barkd path avoids lock-in.
- AI-recipe instincts are correct: versioned in-repo, plain-text fallback, agent never sees keys, expected-output verification.
- WooCommerce-first matches where BTCPay proved demand; Shopify correctly deferred.
- Phased pilots with small mainnet amounts, security review gates between phases, and a Phase 0 spike that can inform redesign — the process is more honest than the claims.