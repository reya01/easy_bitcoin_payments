# Invoices: get paid outside a shop checkout (plan, not built)

**Status: a high-level draft for the owner to approve. No invoice code exists yet.**
Written 22 November 2026, revised the same day: no payment link (see "Why no link"). Nothing here is a promise of dates.

## The idea

Someone who sells a good or a service and has no web shop (a plumber, a designer, a market
stall, a friend you lent money to) opens the **Invoices** tab on their EBP server, types an
amount and a short note, and gets a **payment request**: a Lightning invoice, an Ark
address, and a ready-made message explaining how to pay. They copy it or share it from
their phone by email, text or chat. EBP knows exactly which invoice was paid and when.

## What already exists (checked in the code, 22 November 2026)

- Every payment request already gets its **own** Ark address (the next unused one from the
  wallet's key chain) and its **own** BOLT11 invoice. Payments are matched to a request by
  that address or invoice, never by amount, and the server refuses to hand two open requests
  the same Ark address.
- The watcher, paid / underpaid / overpaid rules, the 3-hour late-payment window, the
  1,000 open-request cap, the payments CSV, phone alerts and the "paid" event already exist.
  Invoices mostly reuse them with a different front door.

## What Bark gives us (Second's docs and the barkd API spec we pin)

| Need | What barkd offers | Consequence |
| --- | --- | --- |
| A unique way to pay one invoice | `POST /lightning/receives/invoice` (amount, optional description, optional `expiry_secs`) | One BOLT11 per invoice. Unique by construction. |
| Lightning invoice lifetime | The Ark server sets it. `expiry_secs` can only shorten it, never extend it. **Measured on Second's server on 15 November 2026: 3 hours** (shorter requests were rewritten up to 3 hours and payments worked the whole way). Second's example server config shows 48 hours, so the number is each server's own setting. | A pasted Lightning invoice is good for hours, not days. |
| Unique Ark way to pay | `POST /wallet/addresses/next` | One Ark address per invoice, which does not expire. |
| One string with both | `POST /wallet/bip321` builds a `bitcoin:` style URI holding the Ark address and, with an amount, a BOLT11, with a label and message | An optional extra for QR codes; same Lightning lifetime. |
| Know when it was paid | `GET /lightning/receives/{id}`, notifications, history | EBP already watches. |
| A reusable "pay me" handle | **Not found.** barkd sends to BOLT12 offers, Lightning addresses and LNURL; its docs show no way to receive on one | A static address would not say which invoice was paid anyway. Watch Second's changelog. |

## Why no payment link

An earlier draft had EBP serve a public payment page per invoice (`/i/<token>`) that makes a
fresh Lightning invoice each time it is opened. That solves Lightning expiry, but:

- it adds a public page on the owner's server that anyone with the URL can use to make
  Lightning invoices on demand (flooding and guessing to defend against);
- if the server or the page is down, the customer cannot pay, while a pasted Ark address
  or an unexpired Lightning invoice does not depend on the page;
- it is more to build, test and keep safe, for a problem the 3-hour window mostly covers.

**Not planned for any version.** Open to revisiting if owners find that "ask me for a fresh invoice" is a real annoyance. The
matching and payment rules would be unchanged; only a front door would be added.

## How it works

**Owner side, a new Invoices tab**
- *New invoice*: amount (sats, or a fiat amount quoted when the invoice is made), a short
  note, an invoice number and a customer name (see below).
- *Identifiers for matching and analysis* (both optional to fill, both saved in the database
  and both columns in the CSV download):
  - **Invoice number.** Filled in by default with the next number, counting up from 1
    (1, 2, 3...). The owner can type their own instead (for example `2026-014` or `INV-0007`).
    It is **unique**: a number already used by another invoice is refused with a plain
    message, so a number always points to exactly one invoice. Cancelled and expired invoices
    keep their numbers (never reused). The default is one more than the highest purely numeric
    number so far, so a typed `INV-0007` does not disturb the counting and a typed `500`
    continues from 501. Gaps are fine. Plain text, trimmed, at most 40 characters.
  - **Customer name.** Free text, at most 100 characters. Not unique on purpose: the same
    customer has many invoices, and the name is what groups them in a spreadsheet. The form
    suggests names used before so one customer is spelled the same way each time.
  - Both are kept only in the server's database and its encrypted backups; never in logs, the
    repository, or phone alerts (an alert says "Invoice 14 paid: 25,000 sats", without the
    customer name, because alerts pass through a push service).
  - Both appear in the Invoices list (search by either), in the canned message (the invoice
    number only, as a reference the customer can quote), and as the first columns of the
    invoices CSV: `invoice_number, customer_name, created_at, paid_at, status, amount_sats,
    paid_sats, fiat_currency, fiat_amount, note`. The same two columns are added to the
    payments CSV (empty for shop orders) so one file can be analysed. Text is handled by the
    existing `csv_cell` rules, so a name that starts with `=` or `+` cannot run as a formula
    in a spreadsheet.
- *The request*, shown on screen and in a canned "how to pay" message with **Copy** and the
  phone's **Share** button. For example:

  > Please pay 25,000 sats to Sam's Plumbing for "Bathroom tap repair".
  >
  > **Pay with Lightning** (any Lightning wallet): paste this invoice into your wallet. It
  > expires on **Sunday 22 November 2026 at 6:42 PM EST (11:42 PM UTC)**. After that time
  > it will not work, and you can ask me for a fresh one.
  >
  > **Pay with Ark** (works after that time too): in an Ark wallet, send exactly 25,000 sats
  > to this address. Ark wallets that work with this server include Noah (Android) and Arké
  > (iPhone), set to the Ark server named below.

  The expiry is the real one, read from the invoice itself after the Ark server has set it
  (not the time EBP asked for, which the server may extend), written out as an exact date
  and time with the time zone in words and as an offset, and also in UTC so a payer in
  another zone can read it. The owner's time zone is taken from their phone's browser.
  The Ark address stays valid after the Lightning invoice expires for as long as the invoice
  is open, and the message says so. The message names the Ark server the wallet must use
  (`ark.second.tech` on a real-money server), because a wallet set to a different Ark server
  cannot pay it (see `KNOWN_ISSUES.md`).
- *Make a fresh Lightning invoice*: one button when it has expired. The invoice keeps the same
  Ark address, and the old Lightning invoice still counts if paid late (the existing 3-hour
  late window).
- *List*: Open, Paid, Part paid, Expired, Cancelled, with the amount received and when.
  Actions: Cancel, Copy again, Download CSV. Phone alert when one is paid.
- EBP does not send email or texts itself: no mail account to set up, nothing for EBP to leak.

**Safety rules**
- Same open-request cap, late-payment window and part / over payment rules as shop orders.
  Bitcoin payments stay final: no refunds.
- A cancelled or expired-and-closed invoice must never be credited by a later payment to its
  old address without the owner seeing it (an alert), the same as shop orders today.
- Fiat invoices fix the sats amount when made. The message says so, and the owner can make a
  fresh one if the price has moved (open question 1).
- Lightning claims depend on wallet notifications (see `KNOWN_ISSUES.md`); the message and the
  Invoices tab use the same plain wording as checkout.

## Versions (proposal, pre-1.0)

| Version | What |
| --- | --- |
| 0.3.x (now) | Hardening and real-money drills. No new features. |
| **0.4** | **Invoices, experimental.** New tab, payment request with copy and share, fresh Lightning invoice button, statuses, cancel, CSV, phone alert. Off until the owner turns it on in Settings. |
| 0.5 | Invoice polish (duplicate an invoice, "valid until" wording, print or save as PDF), a "paid" signed event for people who connect their own tools, and the guided move to the owner's own domain. |
| 0.6 to 0.9 | Release candidates for 1.0: a second independent review of the send path, arm64 builds, reproducible builds, tidy-up from owner use. |
| **1.0** | Declared after the above has run on real money for a while. WooCommerce and invoices. |
| After 1.0 | Shopify, Branta address checks, Lightning and silent-payment payouts, refunds, automatic updates. |

## Phases for 0.4

1. Design sign-off by the owner (this page).
2. Database: an invoice "source" (shop or manual), note, `invoice_number` (unique index) and
   `customer_name`; each Lightning invoice made for an invoice is recorded (the existing
   `ln_invoices` already maps a BOLT11 to an invoice). The next-number counter is derived from
   the stored numbers, not kept separately, so a restore from a backup cannot reuse a number.
3. API and the Invoices tab with copy and share; browser tests at phone width.
4. A real-money drill: one small Lightning invoice (check the expiry time in the message
   against the invoice itself, in the owner's time zone and UTC), one small Ark invoice
   paid from Noah or Arké after the Lightning one expired, a Lightning
   invoice paid after it expired (within the late window), a fresh invoice after expiry, one
   part paid, one cancelled then paid (must not silently credit). Also: two invoices with
   the same typed number (second refused), the default number counting 1, 2, 3 and carrying on
   after a typed `500`, and the CSV opened in a spreadsheet with both columns correct.
5. Release candidate; the owner cuts the tag.

## Open questions for the owner

1. Fiat invoices: fix the sats at creation (simpler, the owner carries the price risk, the
   message says so) or re-quote on a fresh invoice only (recommended)?
2. Business name in the canned message: the store name, or a new setting?
3. Should EBP ever send the message itself by email? Recommended no.
4. Is "mark as paid outside EBP" wanted (cash, bank)? Leaning no for 0.4.
5. Invoice number format: free text with a counting default (as drafted), or numbers only?
   Recommended: free text, so owners can keep an existing numbering scheme.
6. Re-measure the Lightning window on the live Ark server before release; it is the server
   operator's setting and may change.
