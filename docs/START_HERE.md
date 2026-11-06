# Start here — for a shop owner

Plain words, no jargon. If a word is new, it is in the glossary at the bottom.

## What this is, honestly

Easy Bitcoin Payments (EBP) lets your online shop take Bitcoin payments into
**your own wallet**, with no company in the middle and no fees to us.

**Today it is a test version.** You can set it up and take **play-money
(signet) payments** to see how it works. **Do not use real bitcoin yet.**
Two things are not finished, and you should know before you invest time:

- **Shop plugin: WooCommerce only.** There is a WooCommerce plugin
  ([setup guide](WOOCOMMERCE.md)); there is no Shopify app yet. You can also
  try a payment by hand: admin page → Payments → **Try a payment**.
- **Checkout is reachable only by you** in the safest setup (see "Taking real
  customers" below). Opening it to the public needs an HTTPS address.

## What you need

- **A place to run it** — pick ONE:
  - **The computer you are sitting at** (Mac, Windows with WSL, or Linux), left
    on while you test. The simplest: everything opens at `http://localhost:8080`.
  - **A rented cloud server** (Linux). Its address will NOT open from your own
    computer directly (that is deliberate, for safety). You first make a safe
    "tunnel" from your computer: follow Part 5 of
    [MASTER_TEST_WALKTHROUGH.md](MASTER_TEST_WALKTHROUGH.md), then use
    `http://localhost:8080` as above.
- **Docker and git installed.** Docker: <https://docs.docker.com/get-started/get-docker/>.
  On a Linux server put `sudo` in front of the commands below if you see
  "permission denied".
- **A terminal** (Terminal on Mac, the Ubuntu app on Windows, or your server's
  login) and the ability to copy and paste about ten commands. EBP cannot yet
  be set up with zero typing.
- About 90 minutes the first time, mostly waiting: the first start compiles
  the software and is silent for 15–30 minutes. That is normal.
- A pen and paper (for the 12 recovery words — the most important step).
- A phone wallet that can receive signet (play-money) coins. The tested one is
  **Arké** (iPhone). Get free play coins from the signet faucet
  (<https://signet.2nd.dev/>, needs a GitHub login) and wait 5–10 minutes for
  them to arrive.

Step-by-step with every known error and its fix, for a cloud server and an
iPhone: [MASTER_TEST_WALKTHROUGH.md](MASTER_TEST_WALKTHROUGH.md).

## The short version (single-container install)

EBP can run its own wallet engine inside one container, so you never copy
tokens or edit files. (This path is newer and has not yet had its live
signet run — see [SIGNET_DRILL.md](SIGNET_DRILL.md). If you are testing for
us, use it and tell us where it hurts.)

```
git clone https://github.com/reya01/easy_bitcoin_payments.git
cd easy_bitcoin_payments
./scripts/start.sh
```

The script prints a boxed **SETUP CODE** and the address of the setup page.
(If the first start is still compiling it will say so; run it again in a
minute.) Then the setup page walks you through, in this order:

1. **Setup code** — type the code from the box.
2. **Create wallet** — one button.
3. **Recovery words** — 12 words, shown for 60 seconds at a time. Write them
   on paper, twice, keep the copies in different places. Never photograph,
   never type them anywhere else. You will be asked to type three of them
   back to prove you wrote them down correctly.
4. **Name your store, choose an admin password** (10+ characters; it signs
   you in to the admin page and locks your backups) **and paste the address
   of your own wallet** (where the till empties to; you can skip it and add
   it later in Settings).
5. **Two secrets appear once** — the **API key** and the **webhook signing
   secret**. Save both in a password manager. They cannot be shown again.
6. **Finish.** The server restarts once, so the page may not answer for about
   a minute. Then open `/admin` and sign in with your admin password.

## Using Render instead of your own server (experimental)

Render (render.com) can run EBP for you with no terminal, using the
`render.yaml` file in this repository. **This route is untested; try it with
play money only.**

> **⚠ Render needs a PAID plan or EBP will break.** EBP stores your wallet and
> your order history on a disk that must survive restarts. Render only offers
> such a disk on paid services (see <https://render.com/docs/disks>). On a free
> plan there is no disk: **everything is erased every time the service restarts
> or redeploys, including your wallet files.** Choose a paid plan (the one in
> `render.yaml` is "Starter"), check the current price on Render's site, and do
> not switch it to a free plan later.

On Render the setup code is in the service's **Logs** tab (search for
`SETUP CODE`), and you update by pressing **Manual Deploy**; nothing updates
by itself.

## Make a test payment

1. Admin page → **Payments** → **Try a payment** → **Create a test payment
   request** → **Open the checkout page**. It shows a QR code.
2. In your signet wallet, choose Send, scan that QR (from the computer screen)
   and confirm. The checkout page flips to **Paid** within seconds.
3. The payment now shows as paid under **Payments**.

## Keep a backup

Admin page → **Settings** → **Download a backup**. Your browser saves a file
locked with your admin password; copy it somewhere that is not this computer or
server (if your server is remote, you are downloading through your tunnel, so
it lands on your own computer, which is what you want). Download a fresh one
after changing your admin password, because an older file still needs the old
password. Details and how to restore: [RECIPE_BACKUP.md](RECIPE_BACKUP.md).

## How you know it worked

You are done when ALL of these are true:

- [ ] The admin page (`/admin`) opens after you sign in (just a password; no
      username), and its **Home** tab says everything is OK or lists what
      needs attention in plain words.
- [ ] A test payment (admin → Payments → **Try a payment**) shows **Paid** on the checkout page.
- [ ] The same payment appears under **Payments** in the admin page.
- [ ] You downloaded a backup (**Settings → Download a backup**) and stored
      the file somewhere that is not this server.
- [ ] Your recovery words are on paper in two places.

## Getting a padlock (https) address

Customers, and the setup page itself, need a padlock (`https://`) address.
EBP can set that up for you with one extra word. You need a server on the
internet (a small rented one is fine) that accepts visitors on **ports 80 and
443**. Most hosting companies leave these open; if yours has a firewall page,
open both.

**Easiest: no domain name needed.** In the EBP folder, run:

```
./scripts/start.sh --https
```

It finds your server's address and builds a free padlock address from it,
for example `https://203-0-113-7.sslip.io`. It prints that address and your
setup code at the end. Open the address, enter the code, and continue as
before. The first time can take a minute while the padlock is issued.

**With your own domain** (looks nicer, e.g. `pay.myshop.com`): in your
domain's settings, point the name at your server's address, then run:

```
./scripts/start.sh --domain pay.myshop.com
```

The choice is remembered. After that, plain `./scripts/start.sh` (and
upgrades) keep https on and renew the padlock by themselves.

If it prints "the padlock address is not answering yet", the cause is nearly
always closed ports 80/443 or (own domain) a name that does not point at the
server yet. Fix that and run the same command again; nothing is lost.

Until the test version is finished, take only play money. Never set
`EBP_ALLOW_INSECURE_SETUP` on a server reachable from the internet.

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
