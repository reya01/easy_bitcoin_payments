# Shopify (planned, not built)

**Status: research and a written plan only. There is no Shopify code in this
project and the first release does not support Shopify.** WooCommerce is the
supported shop (`WOOCOMMERCE.md`). This page records what we learned so the next
step is a decision, not another round of research.

Things marked **[checked]** were read on Shopify's developer documentation
(shopify.dev) when this was written. Things marked **[unverified]** are
assumptions that must be tested on a Shopify development store before anyone
relies on them. Shopify changes these rules; re-check before building.

## The hard part: Shopify controls who may add a payment method

- **[checked]** Payment extensions (the way an app adds a real payment option to
  Shopify checkout) can only be built by **approved Payments Partners**. A
  public listing needs a separate application and review, and partners must
  accept Shopify's Payments Partner requirements, including revenue-share
  agreements.
- **[checked]** The docs say building a *custom* payments extension is limited to
  eligible **Shopify Plus** merchants.
- **[checked]** Cryptocurrency is listed as a supported payment-method type, and
  there is an "offsite" extension type (the buyer is redirected to a page the
  app hosts, then sent back). That is the shape EBP already has.
- **[checked]** A community answer says a plain custom app cannot redirect
  checkout to an outside gateway; the approved route is a payments app, or a
  manual payment method. (A forum reply, not official, but consistent with the
  docs above.)

So the best experience (Bitcoin shown next to cards at checkout) is gated by a
partner approval and an agreement with Shopify, not by engineering.

## How EBP would map onto the official "offsite" flow

**[checked]** for the Shopify side, our side is a design:

| Shopify does | EBP would do |
|---|---|
| POSTs the order to `payment_session_url`; expects a `redirect_url` back | Create an invoice (`POST /api/v1/invoices`, idempotent per order), return our `/i/{id}` checkout page. Pass Shopify's return link as `return_url` (already supported). |
| Buyer pays on the EBP page | Same as today |
| App must tell Shopify the result | On `invoice.paid` call `paymentSessionResolve`. On expiry call `paymentSessionReject`. While waiting, `paymentSessionPending` with an expiry. |
| POSTs to `refund_session_url` (required) | Bitcoin refunds are manual (we do not know where to send them). Reject with a clear merchant message, and show the merchant the amount and order. |

Requirements that follow: a server Shopify can reach holding Shopify API
credentials, handling Shopify's retries, and HMAC checks on everything Shopify
sends.

## The sovereignty problem

EBP's promise is that the merchant runs their own server and holds their own
keys. An official Shopify payments app normally is a **central app** many
merchants install, run by the app's developer. That means:

- A project-run service would sit between every merchant's Shopify store and
  their EBP. It would hold Shopify access tokens (not funds), be a single
  point of failure and trust, and be a vendor, which is what we said we would
  avoid.
- Doing it per merchant (each merchant's own app) is the part Shopify limits to
  Plus and approved partners.

If we ever do this, the central piece should be as thin as possible: it only
relays "session started" and "paid" between Shopify and the merchant's own EBP,
never sees the merchant's wallet, and the merchant's EBP signs every message.

## Options

| | A. Official offsite payments app | B. Manual payment method + merchant-owned helper | C. Payment link / draft-order invoices | D. WooCommerce only for now |
|---|---|---|---|---|
| Buyer experience | Best: Bitcoin at checkout | Okay: order is placed "payment pending", buyer then pays from a page or email | Poor: merchant sends a link by hand | n/a |
| Needs Shopify approval | **Yes** (Payments Partner, revenue share) | No **[unverified]** | No | No |
| Plans | Public listing: all merchants. Custom: Plus only | All plans **[unverified]** | All plans | n/a |
| Who runs the server | A central app (conflicts with our goals) or each merchant | The merchant's own EBP | The merchant's own EBP | n/a |
| Effort | Large + business process | Medium | Small | None |
| Fits "easy for beginners" | Yes, for the merchant | Mostly | No | Yes (WooCommerce) |

### What option B could look like (the only one we can try without permission)

1. The merchant adds a **manual payment method** called "Bitcoin" in Shopify.
   The order is created as payment pending. **[unverified]**: how much the
   customer sees afterwards, and whether the thank-you page can show a
   per-order link (a thank-you-page extension would need an app; whether a
   merchant-owned app can ship one on every plan is not confirmed).
2. The merchant creates a **custom app inside their own store** (no store
   review) with the narrowest scopes, probably read/write orders only.
   **[unverified]** exact scope names and whether it can read the order from a
   `orders/create` webhook on a basic plan.
3. EBP gets a small "Shopify connection" screen: the store address, the app's
   token, and Shopify's webhook secret. On `orders/create` for a Bitcoin order
   it creates an invoice and emails or shows the link (via order metafield or
   the notification email). On `invoice.paid` it marks the order paid through
   the Admin API.
4. The shop-side tripwire idea from WooCommerce (a check that does not depend on
   the EBP server) has no direct equivalent here and needs its own design.

Risks: the Shopify token is another secret on the EBP server (stored encrypted
like other secrets); a stolen token could mark orders paid, so scopes must stay
minimal and we need the same amount-mismatch guard as WooCommerce (an order
total that changed after the invoice was made must not be auto-paid).

## Recommendation

1. Ship the first release with **WooCommerce only**. Say so plainly (README and
   START_HERE already do).
2. Before spending engineering time, answer two questions that cost nothing but
   a day of someone's time:
   - **Ask Shopify** (Partner support) whether a self-hosted, open-source,
     merchant-run payment connector can be an approved payments app without a
     central service, and what the revenue share would be for a Bitcoin offsite
     method.
   - **Try option B by hand** on a free Partner development store: is the manual
     payment method flow acceptable for a buyer, and what can the thank-you
     page show?
3. Decide after those two answers. If Shopify requires a central service,
   that is a product decision about vendor lock-in, not a coding task.

## Things only the project owner can do

- Create a free Shopify Partner account and a development store (needed for any
  test).
- Contact Shopify Partner support with the question above.

## Sources

- Shopify, "Extensions for payments": https://shopify.dev/docs/apps/build/payments
- Shopify, "Build an offsite payments extension": https://shopify.dev/docs/apps/build/payments/offsite/use-the-cli
- Shopify Community, custom app redirect question: https://community.shopify.com/t/use-custom-app-to-redirect-payment-requests-to-my-payment-gateway-api/399474
