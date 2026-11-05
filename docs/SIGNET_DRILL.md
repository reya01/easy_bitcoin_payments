# Signet drill (the live checks CI cannot do)

Everything here uses **signet** (test coins with no value). Do it on a throwaway
deployment, never on a wallet that holds real money. About 60-90 minutes, mostly
waiting for blocks. Print or keep this open next to the setup guide
(`docs/MASTER_TEST_WALKTHROUGH.md` covers creating the server).

CI already proves the logic against a *mock* barkd. This drill proves the parts
a mock cannot: that the real barkd 0.8.0 answers with the shapes we parse, and
that the first-run experience works end to end.

**Record the result of every step** (PASS / FAIL / notes) in the table at the
bottom and, for any FAIL, save the exact error text and the response body.
Never paste recovery words, the setup code, API keys or tokens into a report.

## 0. Before you start

- Deploy the single-container image (the default `Dockerfile` target) with
  `EBP_ONCHAIN_RAIL_ENABLED=true`, signet network, over HTTPS (setup refuses
  plain http off localhost).
- Have a signet faucet and, ideally, a second signet wallet to send from
  (so you can test underpay and overpay).
- Have a test shop webhook receiver (any request-catcher URL works).

## 1. First run (managed barkd)

| # | Do | Expect |
|---|----|--------|
| 1.1 | Open the site address | Setup page loads; the https check passes |
| 1.2 | Enter the one-time setup code from the server log | Accepted; a wrong code is refused |
| 1.3 | Create the wallet | Wallet created; **the server restarts itself once** and reconnects without any token copying (watch the log: one planned exit, then "connected") |
| 1.4 | Recovery words screen | Words appear for ~60 s; hiding wipes them; copy is blocked |
| 1.5 | Answer the 3-word quiz wrongly, then correctly | Wrong answers are refused **by the server**; right ones unlock "I saved them" |
| 1.6 | Set the admin password | Admin page login works; wrong password is refused and rate-limited after 10 tries |

**FAIL signals:** server loops restarting; token errors; words still shown after
hide; quiz accepted wrong words.

## 2. Ark payments (the main rail)

| # | Do | Expect |
|---|----|--------|
| 2.1 | Make a test order in the shop / Connect tab | Invoice shows an Ark address and QR |
| 2.2 | Pay the exact amount over Ark | Paid within one poll interval; webhook `invoice.paid` arrives once |
| 2.3 | Pay **less** | Status underpaid; remaining amount shown |
| 2.4 | Pay **more** | Paid; surplus visible, nothing lost |
| 2.5 | Pay after the invoice expired | Not credited; appears under Alerts as unmatched, with the amount |
| 2.6 | Restart the server mid-payment | Nothing lost or doubled after restart |

Also confirm: `GET /api/v1/history` returns the shape the parser expects (the
payment is attributed to the right invoice, not parked as unmatched).

## 3. On-chain payments (off by default; the main unknown)

The on-chain path reads `GET /onchain/transactions` and matches outputs by
script. Its response shape is **unverified against a live daemon**.

| # | Do | Expect |
|---|----|--------|
| 3.1 | Pay the invoice's on-chain address, do not wait for a block | In the mempool: **not** credited; invoice still pending |
| 3.2 | Wait for 1 confirmation | Credited exactly once; webhook once |
| 3.3 | Pay on-chain with a replaceable (RBF) transaction, then replace it with a higher fee | Credited once, for the amount actually mined |
| 3.4 | Expire an invoice **after** broadcasting but **before** a block | Still credited (first seen in time) |
| 3.5 | Send two payments to the same address | Both counted; no double credit of either |

Capture one real `/onchain/transactions` response (redact nothing needed; it
holds no secrets) and compare to the `confirmation: {height, hash}` and `tx`
(raw hex) fields in `src/wallet/barkd.rs`. If the shape differs, that is the
bug to fix before enabling the rail.

## 4. Admin page, alerts, webhooks

| # | Do | Expect |
|---|----|--------|
| 4.1 | Log in on the admin page; Home tab | Balances load; version row shows |
| 4.2 | Connect tab: save a webhook address, press the test button | `webhook.test` arrives at your receiver, signature verifies |
| 4.3 | Rotate the API key | Old key stops working at once |
| 4.4 | Log out; open an admin API call without logging in | Refused |

## 5. Upgrade and backup

| # | Do | Expect |
|---|----|--------|
| 5.1 | Deploy a build that adds a database migration (or use a copy of the data) | A file appears in `backups/` before the change; the app starts |
| 5.2 | Start an OLDER build against the newer data | Refuses to start with the plain "newer version" message; data untouched |
| 5.3 | Restore the backup copy and start the older build | Works |
| 5.4 | Update banner | Appears only after you publish a newer release; **nothing installs by itself** |

## 6. Settlement and exits (do last; the riskiest)

| # | Do | Expect |
|---|----|--------|
| 6.1 | Trigger a sweep to your own Ark wallet (small amount) | Sent once; record the real response body (its shape is unpinned) |
| 6.2 | Check the exit reserve warning by lowering funds | Warning shows; a sweep that would breach it is refused |
| 6.3 | Renew expiring coins (VTXO refresh) | Succeeds; the request body matches the pinned OpenAPI |

## Result table

| Step | PASS / FAIL | Notes (error text, response body, log lines) |
|------|-------------|----------------------------------------------|
| 1.x  |             |                                              |
| 2.x  |             |                                              |
| 3.x  |             |                                              |
| 4.x  |             |                                              |
| 5.x  |             |                                              |
| 6.x  |             |                                              |

## After the drill

- Fix every FAIL before turning on the on-chain rail by default.
- Trim the defensive parsing in `src/wallet/barkd.rs` for shapes the drill
  proved are not real (listed in `docs/REVIEW_2026-11-05.md`).
- Delete the throwaway server so billing stops (`MASTER_TEST_WALKTHROUGH.md`
  Part 8).
