# Invoices: get paid outside a shop checkout (plan, not built)

**Status: a high-level draft for the owner to approve. No invoice code exists yet.**
Written 22 November 2026 after reading Second's Bark documentation and checking how EBP
already issues addresses. Nothing here is a promise of dates.

## The idea

Someone who sells a good or a service, and has no web shop (a plumber, a designer, a market
stall, a friend you lent money to), opens the **Invoices** tab on their EBP server, types an
amount and a short description, and gets a **link**. They send it by email, text, chat, or
show it as a QR code. The person who owes the money opens the link on their phone and pays
in bitcoin. EBP knows exactly which invoice was paid, by whom (the owner's own label), and
when.

## Why this fits what EBP already has

Checked in the code on 22 November 2026:

- Every payment request already gets its **own** Ark address (the next unused one from the
  wallet's key chain, `POST /wallet/addresses/next`) and, for Lightning, its **own** BOLT11
  invoice. Payments are matched to a request by that address or invoice, never by amount.
  The server also refuses to hand two open requests the same Ark address.
- The watcher, paid / underpaid / overpaid / late-payment rules, the 1,000 open-request
  cap, the signed "paid" event, the payments CSV, phone alerts and the checkout page all
  exist. Invoices mostly reuse them with a different front door.

## What Bark gives us (from Second's docs and the barkd API spec we pin)

| Need | What barkd offers | Consequence |
| --- | --- | --- |
| A unique way to pay one invoice | `POST /lightning/receives/invoice` (amount, optional description, optional `expiry_secs`) | One BOLT11 per invoice or per page opening. Unique by construction. |
| Lightning invoice lifetime | `expiry_secs` **cannot exceed the Ark server's own invoice expiry** | A BOLT11 cannot stay payable for days. A shared invoice must not rely on one long-lived BOLT11. |
| Unique Ark way to pay | `POST /wallet/addresses/next` | One Ark address per invoice. Lives as long as the invoice. |
| One link/QR with both | `POST /wallet/bip321` builds a BIP 321 URI: Ark address, plus a BOLT11 when an amount is given, with `label` and `message` | Good for the QR, but it inherits the short BOLT11 lifetime. |
| Know when it was paid | `GET /lightning/receives/{id}`, notifications (long poll or websocket), history | EBP already watches; no new wallet feature needed. |
| A reusable "pay me" handle | **Not found.** barkd *sends* to BOLT12 offers, Lightning addresses and LNURL, but its docs show no way to *receive* on one | A static Lightning address would not say which invoice was paid anyway. Watch Second's changelog (0.8 and 0.9 added sending only). |
| Second's own products | Bark Pay (a developer payment processor with a hosted checkout) and a BTCPay Server plugin | Neither is the owner-first "send a link" experience. Worth rechecking their docs for shareable invoices before building. |

Not found does not mean impossible: the docs pages for Bark Pay were not reachable from the
tools used for this draft, and the blog post on Bark MCP says only that agents can be paid,
with no invoice detail.

## Options

1. **A. One long BOLT11 per invoice.** Simplest, wrong: the Ark server caps its lifetime,
   so the link dies in minutes or hours and the customer sees "expired".
2. **B. A payment page per invoice that mints a fresh BOLT11 each time it is opened
   (recommended).** The link is `https://<your server>/i/<long random token>`. The page
   shows who is asking, how much and what for, with a Lightning QR (fresh, a 20-minute
   countdown like checkout) and an Ark option (the invoice's own address, unchanging).
   The link works for as long as the invoice is open (days or weeks). Every BOLT11 the page
   mints is recorded against the invoice, so a late Lightning payment still lands on the
   right one.
3. **C. A BIP 321 URI or QR only.** Fine as an extra for in-person use, not for sharing:
   same short BOLT11 problem and a bad experience in email.
4. **D. A static Lightning address or BOLT12 offer for the whole server.** Not available
   to receive with barkd today, and it would not identify the invoice.
5. **E. Ark address only.** Unique per invoice and long-lived, but fewer payers can pay it
   today. Keep as the second button, not the only one.

## How it would work (option B)

**Owner side, a new "Invoices" tab**
- *New invoice*: amount (sats, or a fiat amount), description, optional label for who it is
  for (typed by the owner, stored only in the server's database and its encrypted backups,
  never in logs or this repository), optional "valid until".
- *Share*: the phone's own share sheet (so it goes out from the owner's own email, text or
  chat app), Copy link, and a QR. EBP does not send email or texts itself: no mail
  account to set up, nothing for EBP to leak.
- *List*: Open, Paid, Part paid, Expired, Cancelled, with the amount received, when, and by
  which method. Actions: Cancel, Extend, Copy link, Download CSV.
- A phone alert when one is paid (the existing alerts).

**Payer side, the page at the link**
- Reuses the checkout look: the amount, the description, the business name, Lightning first
  and Ark second, "type this exact amount" help for Ark, a clear Paid screen.
- No account, no sign-in, nothing to install.

**Safety rules**
- The token is long and random; there is no public list of invoices, so nobody can browse
  them. The description warns that anyone with the link can read it.
- The page is public, so minting a Lightning invoice is rate-limited per invoice and per
  address, and an unexpired BOLT11 is reused rather than making a new one on every refresh.
- Fiat-priced invoices are quoted when the page is opened and held for the 20-minute
  window, so a link opened two weeks later is not priced at an old rate (open question 1).
- Manual invoices use the same open-request cap, the same late-payment window and the same
  rules for part and over payments as shop orders. Bitcoin payments stay final: no refunds.
- Lightning claims depend on wallet notifications (see `KNOWN_ISSUES.md`); the payer page
  uses the same wording as checkout.

## Versions (proposal, pre-1.0)

| Version | What |
| --- | --- |
| 0.3.x (now) | Hardening and real-money drills. No new features. |
| **0.4** | **Invoices, experimental.** New tab, link, payer page, statuses, cancel, CSV, phone alert. Off until the owner turns it on in Settings. |
| 0.5 | Invoice polish: "valid until" and overdue wording, extend, duplicate an invoice, print or save-as-PDF page, a "paid" signed event for people who connect their own tools. The guided move to the owner's own domain, because a link on your own address looks far more trustworthy to a customer. |
| 0.6 to 0.9 | Release candidates for 1.0: second independent review of the send path, arm64 builds, reproducible builds, tidy-up from owner use. |
| **1.0** | Declared once the above has run on real money for a while. WooCommerce and invoices. |
| After 1.0 | Shopify, Branta address checks, Lightning and silent-payment payouts, refunds, automatic updates. |

0.4 is the right size because the hard parts (matching payments to a request, late and
part payments, alerts) are already built and tested on real money; the work is a new tab,
one public page and its safety limits.

## Phases for 0.4

1. Design sign-off by the owner (this page).
2. Database: an invoice "source" (shop or manual), description, label, share token; a table
   of every BOLT11 minted for an invoice (the existing `ln_invoices` already maps a
   BOLT11 to an invoice).
3. API and the public page, with the rate limits above and tests for the unauthenticated
   page (guessing, flooding, expired, cancelled).
4. The Invoices tab and share sheet; browser tests at phone width.
5. A real-money drill: one small Lightning invoice, one small Ark invoice, one paid after the
   page was reopened a day later, one part paid, one cancelled then paid (must not credit).
6. Release candidate; owner cuts the tag.

## Open questions for the owner

1. Fiat invoices: re-quote when the link is opened (recommended, safer for the owner) or fix
   the amount at creation (simpler, but the owner carries the price risk)?
2. Should an invoice be allowed to stay open with no end date, or always have one?
3. Business name on the payer page: take it from the store name, or a new setting?
4. Should EBP ever send the link itself by email? Recommended no: it needs a mail account and
   makes EBP a sender of messages in the owner's name.
5. Is "mark as paid outside EBP" wanted (cash, bank)? It helps record keeping but is a way to
   mislead yourself in the CSV; leaning no for 0.4.
