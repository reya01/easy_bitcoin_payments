# Signet drill (the live checks CI cannot do)

Everything here uses **signet** (test coins with no value). Do it on a throwaway
deployment, never on a wallet that holds real money. About 60-90 minutes, mostly
waiting for blocks. Print or keep this open next to the setup guide
(`docs/START_HERE.md` covers creating the server).

CI already proves the logic against a *mock* barkd. This drill proves the parts
a mock cannot: that the real barkd 0.8.0 answers with the shapes we parse, and
that the first-run experience works end to end.

**Record the result of every step** (PASS / FAIL / notes) in the table at the
bottom and, for any FAIL, save the exact error text and the response body.
Never paste recovery words, the setup code, API keys or tokens into a report.

## 0. Pre-flight checklist (do this the day before)

Tick every box before you start the clock. Anything unticked wastes the drill.

**Accounts and things to have ready**
- [ ] A throwaway machine or cloud server with Docker and git (never one that
      holds real money or other services). Time: block out 90 minutes.
- [ ] A way to open the setup page securely. Pick ONE: run on the computer you
      are sitting at and use `http://localhost:8080` (simplest), OR a server +
      the SSH tunnel in `START_HERE.md` Part 5, OR a real
      `https://` address. Setup refuses plain http anywhere else. If you can,
      also try the real-https route once: it switches the session cookie to
      the `__Host-` form (step 1.7), which localhost cannot test.
- [ ] A signet wallet on your phone (Arké) with a few thousand test sats, and
      ideally a second signet wallet to send from (underpay/overpay). Faucet:
      <https://signet.2nd.dev/> (GitHub login; coins take 5-10 minutes).
- [ ] A webhook receiver URL (any request-catcher) for steps 2.2 and 4.2.
- [ ] Paper and pen for the recovery words; a password manager for the admin
      password, API key, webhook secret and the encryption key.
- [ ] A place to write results: this file's table (copy it into a note).

**Install exactly as a customer would** (this tests the docs too)
- [ ] Follow `docs/START_HERE.md` word for word, including the `.env` lines.
      Note anything you had to guess or that did not match the screen.
- [ ] For the on-chain steps set `EBP_ONCHAIN_RAIL_ENABLED=true` in `.env`
      (and restart with `./scripts/start.sh`).

**While you test**
- [ ] Run `./scripts/drill_capture.sh` right after each of: 1.3 (wallet
      created), 2.2 (first Ark payment), 3.2 (first confirmed on-chain
      payment), 6.1 (sweep). It saves barkd's real answers and the barkd data
      folder listing into a file (no secrets in it); keep all of them.
- [ ] Take a screenshot (words hidden!) or note of anything confusing.

**What this drill is for.** It answers the things nothing else can: the real
shape of an Ark payment in barkd's history (step 2.2), the real on-chain
transaction shape (3.x), what barkd's data folder really contains (5.6), what a
sweep returns (6.1), and whether a beginner can follow `START_HERE.md`.

## 1. First run (managed barkd)

| # | Do | Expect |
|---|----|--------|
| 1.1 | Open the site address | Setup page loads; the https check passes |
| 1.2 | Enter the one-time setup code from the server log | Accepted; a wrong code is refused |
| 1.3 | Create the wallet | Wallet created; **the server restarts itself once** and reconnects without any token copying (watch the log: one planned exit, then "connected") |
| 1.4 | Recovery words screen | Words appear for ~60 s; hiding wipes them; copy is blocked |
| 1.5 | Answer the 3-word quiz wrongly, then correctly | Wrong answers are refused **by the server**; right ones unlock "I saved them" |
| 1.6 | Set the admin password | Admin page login works; wrong password is refused and rate-limited after 10 tries |
| 1.7 | (https only) sign in and check the browser's cookie list | Cookie is named `__Host-ebp_session`; signing in and out works; on http/localhost it is `ebp_session` |

**FAIL signals:** server loops restarting; token errors; words still shown after
hide; quiz accepted wrong words.

## 2. Ark payments (the main rail)

| # | Do | Expect |
|---|----|--------|
| 2.1 | Admin page → Payments → **Try a payment** (or a test order from your shop) | Invoice shows an Ark address and QR |
| 2.2 | Pay the exact amount over Ark | Paid within one poll interval; webhook `invoice.paid` arrives once |
| 2.3 | Pay **less** | Status underpaid; remaining amount shown |
| 2.4 | Pay **more** | Paid; surplus visible, nothing lost |
| 2.5 | Pay after the invoice expired | Not credited; appears under Alerts as unmatched, with the amount |
| 2.6 | Restart the server mid-payment | Nothing lost or doubled after restart |

**Run `scripts/drill_capture.sh` now.** In the saved `GET /api/v1/history`
answer, find the entry for your payment. Expected (see
`plain_ark_address_receive_is_credited_using_its_output_vtxo_as_the_dedupe_key`
in `src/wallet/barkd.rs`): it has an `output_vtxos` list and the receiving
address, and the invoice is credited. If your payment is in the history but
the invoice was NOT credited, the shape differs: that is the bug to fix first.

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
| 5.5 | Settings → **Download a backup** (admin password); then `docker compose cp` the file into the container and run `ebp backup verify` on it (see RECIPE_BACKUP.md) | File downloads; verify lists `ebp.sqlite3` and, in single-container mode, `barkd/...` files; a wrong password is refused |
| 5.6 | Compare the `barkd/...` names in that list with the data-folder listing from `drill_capture.sh` | Every wallet file is present. If barkd keeps its database in a file the backup skipped (or in a WAL form that did not copy), write down the name: the backup code needs a fix |
| 5.7 | `ebp backup restore` into a new folder, then start a SECOND throwaway stack on those files | The restored stack shows the same balance and payment history. (The encryption key from `.env` must be put back first) |
| 5.4 | Update banner | Appears only after you publish a newer release; **nothing installs by itself** |

## 6. Settlement and exits (do last; the riskiest)

| # | Do | Expect |
|---|----|--------|
| 6.1 | Trigger a sweep to your own Ark wallet (small amount) | Sent once; record the real response body (its shape is unpinned) |
| 6.2 | Renew expiring coins (VTXO refresh) | Succeeds; the request body matches the pinned OpenAPI |

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
  proved are not real (listed in `docs/dev/REVIEW_2026-11-05.md`).
- Delete the throwaway server so billing stops (`START_HERE.md`
  Part 8).
