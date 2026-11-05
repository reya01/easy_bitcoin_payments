# Start here — for a shop owner

Plain words, no jargon. If a word is new, it is in the glossary at the bottom.

## What this is, honestly

Easy Bitcoin Payments (EBP) lets your online shop take Bitcoin payments into
**your own wallet**, with no company in the middle and no fees to us.

**Today it is a test version.** You can set it up and take **play-money
(signet) payments** to see how it works. **Do not use real bitcoin yet.**
Three things are not finished, and you should know before you invest time:

- **No shop plugin yet.** There is no WooCommerce / Shopify button. Your shop
  (or a developer) has to call EBP to create payment requests, so there are no
  automatic orders. You can still try a payment by hand: admin page →
  Payments → **Try a payment**.
- **Checkout is reachable only by you** in the safest setup (see "Taking real
  customers" below). Opening it to the public needs an HTTPS address.

## What you need

- A computer you can leave on, or a small rented cloud server, with Docker.
  About 90 minutes the first time, mostly waiting.
- The ability to copy and paste about ten commands. EBP cannot yet be set up
  with zero typing.
- A pen and paper (for the 12 recovery words — the most important step).
- A phone wallet that can receive signet coins, to make a test payment.

Step-by-step with screenshots-in-words for a cloud server and an iPhone:
[MASTER_TEST_WALKTHROUGH.md](MASTER_TEST_WALKTHROUGH.md).

## The short version (single-container install)

EBP can run its own wallet engine inside one container, so you never copy
tokens or edit files. (This path is newer and has not yet had its live
signet run — see [SIGNET_DRILL.md](SIGNET_DRILL.md). If you are testing for
us, use it and tell us where it hurts.)

```
git clone https://github.com/reya01/easy_bitcoin_payments.git
cd easy_bitcoin_payments
COMPOSE_FILE=docker-compose.managed.yml ./scripts/start.sh
```

The script prints a boxed **SETUP CODE** and the address of the setup page.
Then the setup page walks you through, in this order:

1. **Setup code** — type the code from the box.
2. **Create wallet** — one button.
3. **Recovery words** — 12 words, shown for 60 seconds at a time. Write them
   on paper, twice, keep the copies in different places. Never photograph,
   never type them anywhere else. You will be asked to type three of them
   back to prove you wrote them down correctly.
4. **Name your store and choose an admin password** (10+ characters). The
   admin password signs you in to the admin page (`/admin`) later.
5. **Two secrets appear once** — the **API key** and the **webhook signing
   secret**. Save both in a password manager. They cannot be shown again.

## How you know it worked

You are done when ALL of these are true:

- [ ] The admin page (`/admin`) opens after you sign in, and its **Home**
      checklist has no red items.
- [ ] A test payment (admin → Payments → **Try a payment**) shows **Paid** on the checkout page.
- [ ] The same payment appears under **Payments** in the admin page.
- [ ] You downloaded a backup (**Settings → Download a backup**) and stored
      the file somewhere that is not this server.
- [ ] Your recovery words are on paper in two places.

## Taking real customers (later)

Customers need to reach your checkout over `https://`. That means a domain
name plus either a tunnel service or a reverse proxy in front of EBP, and
setting `EBP_PUBLIC_BASE_URL` to that address. Until the test version is
finished, do this only with play money. Never set `EBP_ALLOW_INSECURE_SETUP`
on a server reachable from the internet.

## Updating

EBP never updates itself. When a new version exists, the admin Home page
shows a notice and exact steps; you decide when. See
[UPGRADING.md](UPGRADING.md). A copy of your data is taken automatically
before every upgrade.

## If something goes wrong

- The admin Home page lists problems in plain words, worst first.
- Known problems and fixes: [KNOWN_ISSUES.md](KNOWN_ISSUES.md).
- Backup and restore: [RECIPE_BACKUP.md](RECIPE_BACKUP.md).

## Glossary

- **Signet** — a practice Bitcoin network with worthless coins.
- **Wallet / till** — the small wallet EBP uses to receive payments. EBP
  moves its contents to *your own* wallet on a schedule; the till is kept small.
- **Recovery words (mnemonic)** — 12 words that can rebuild the till wallet.
  Anyone with them can take its money.
- **Ark** — the payment technology EBP uses. **ASP / Ark server** is the
  service that coordinates it; you can leave this at the default.
- **barkd** — the wallet program (by Second) that EBP talks to.
- **API key** — the password your *shop* uses to ask EBP for payments.
- **Webhook** — a message EBP sends your shop when an order is paid. The
  **signing secret** lets your shop check the message really came from EBP.
- **Backup file** — an encrypted copy of your history and settings, opened
  with your admin password.
