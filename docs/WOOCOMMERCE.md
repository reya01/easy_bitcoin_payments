# Connect your WooCommerce shop

Time needed: about 10 minutes. You need your payment server running with a
padlock (https) address, and admin access to your WordPress site.

**Your shop must be reachable from the internet.** The payment server tells your
shop about payments by calling its web address, and it refuses to call addresses
that only work on your own network (`localhost`, `127.0.0.1`, `192.168.…`,
`10.…`). A WordPress running only on your computer (for example "Local") will not
get payment notices; use a shop with a public address.

> **These steps are a guide, not an exact script.** Every hosting company shows WordPress a little differently: menu names, where plugins are uploaded, and what your plan allows can all vary. These steps use the standard WordPress and WooCommerce menus.
> If something does not match what you see, or you get stuck, copy this page
> into an AI assistant (or give it this link: <https://github.com/reya01/easy_bitcoin_payments/blob/main/docs/WOOCOMMERCE.md>), tell it which
> service you use, and ask it to help you work out the equivalent steps.
> **Never give an AI, or anyone, your passwords, API key, signing secret or
> recovery words.** Describe what you see instead, or hide those values first.

**Test version:** use play-money (signet) only for now. Do not take real
bitcoin yet.

## Two places, kept apart in these steps

You will flip between two different websites, and some names look alike. Every
step below says which one it means:

- **Your payment server** is the Easy Bitcoin Payments admin page, at your server's
  address followed by `/admin` (for example `https://pay.myshop.com/admin`).
  Its tabs (Home, Payments, Alerts, Connect, Settings) are in a bar at the very
  bottom of the screen.
- **Your WooCommerce site** is the WordPress admin of your shop (the
  WooCommerce.com or WordPress dashboard where you installed the plugin).

## 1. Install the plugin

1. Get `ebp-bitcoin-woocommerce.zip`.
   - **While testing (no release has the plugin yet):** the project owner sends
     you the zip, or you download it from GitHub: **Actions**, open the newest
     green **CI** run, scroll to **Artifacts** at the bottom and download
     **plugin-zip** (you need to be signed in to GitHub; the download is a zip
     that contains the plugin zip, so unzip it once first). Artifacts are kept
     for 14 days.
   - **From the first release candidate on:** open the release you are testing
     (for example `v0.1.0-rc1`) on the project's **Releases** page and look
     under **Assets**. Do not use the "latest release" shortcut: GitHub skips
     pre-releases such as `rc1` there.
2. **On your WooCommerce site:** **Plugins → Add New → Upload Plugin**, choose the zip, then
   **Install Now** and **Activate**.
3. **On your WooCommerce site:** go to **WooCommerce → Settings → Payments**. Newer WooCommerce shows a list
   of payment methods: press **Manage** next to **Bitcoin (Easy Bitcoin
   Payments)** (older versions open it by clicking its name). Its settings page
   is where you fill in everything below. Leave **Enable** unticked until
   section 2 is done.

## 2. Fill in three things

Open both in their own browser tabs and keep them side by side:

- **Your payment server:** your server's address followed by `/admin`. Sign in with
  your admin password, then tap **Connect** in the bar at the very bottom of the
  screen. Its heading says "Connect your shop".
- **Your WooCommerce site:** **WooCommerce → Settings → Payments**, then **Manage**
  next to **Bitcoin (Easy Bitcoin Payments)**. The same three steps are listed at
  the top of it.

| Box on your WooCommerce site (plugin settings) | Where to get it, on your payment server |
| --- | --- |
| Server address | Card 1 on your payment server's **Connect** tab (your padlock address) |
| API key | The key you were shown during setup (you saved it in your password manager). Only if you lost it: card 2 on your payment server's **Connect** tab, **Make a new API key** (the old one stops working at once; shown once, copy it right away) |
| Signing secret | Card 3 on your payment server's **Connect** tab: appears once when you save the notification address |

### The signing secret, step by step

The signing secret is how your shop knows a "payment received" message really
came from your payment server. You get it by telling the server your shop's
notification address. Do this in order, with both pages open:

1. **On your WooCommerce site**, on the plugin's settings page, find **Your shop's
   notification address**. It is a web address under the boxes, for example
   `https://myshop.com/?wc-api=ebp_bitcoin`. Select it and copy it.
2. **On your payment server**, on the **Connect** tab, scroll to card **3. Payment
   notifications to your shop**. Paste the address into the box **Your shop's
   webhook address**.
3. **On your payment server:** tap **Save address**. A box appears that says **Signing secret, shown once**.
   Tap **Copy** under it, and also save it in your password manager (the same
   place as your API key). The box disappears when you leave the page, so do this
   and the next step right away.
4. **On your WooCommerce site**, on the plugin's settings page, paste it into the **Signing secret**
   box. It shows dots as you paste; that is normal.
5. **On your WooCommerce site:** tick **Enable** and press **Save changes** at the bottom of the plugin's settings page.
   Green "connected to your payment server" means it worked. A red message says
   what to fix.

**No secret box appeared in step 3?** Your payment server only shows a secret the
first time you save the address. If you saved the address before, tap **New
signing secret** (next to Save address, on your payment server) instead. It makes a fresh secret and shows it
once. Paste that one into the plugin.

**Lost it, or it says the signature does not match?** Do the same thing: tap
**New signing secret** on your payment server, copy the new one, paste it into the plugin on your WooCommerce site, and save.
The old secret stops working at once.

Never paste the signing secret anywhere except that one plugin box.

## 3. Try it

**On your payment server:** on the **Connect** tab press **Send a test notification**
(nothing is charged). Then **on your WooCommerce site** place a small test order on your own shop and pay it with a signet wallet.
The order should move from **On hold** to **Processing** by itself within a few
seconds of the payment.

## What your customers see

They pick **Bitcoin** at checkout and land on your payment server's checkout
page (Ark; Lightning when your Ark server offers it; on-chain only if you
turned it on). Underpaying keeps the order on hold for a
short top-up window; if the invoice expires unpaid, the order is cancelled and
stock is released. If a payment arrives after the order was cancelled, the
order notes say so, so you can decide what to do (see **Payments that need
you** on your payment server's dashboard).

## Warnings the plugin gives you

The plugin checks on its own, from your shop, every hour, even if your payment
server is down or wrong. It never changes orders; it only warns you with a red
notice in the WordPress admin and one email a day per problem:

- **The payment server has not answered for three checks in a row.**
- **Five or more Bitcoin orders in the last 7 days and none paid.** Check that
  the server is up and your notification address is right.

If a customer pays but the order total changed after checkout (for example you
edited the order), the order is **not** marked paid. It gets a note saying a
payment arrived for a different amount; check your wallet and complete or
refund it by hand.

## Safety notes

- Orders are only marked paid after the plugin asks **your own server**
  whether the invoice is really paid. A forged or replayed notice cannot mark
  an order paid.
- The plugin only talks to the server address you typed in. No third party is
  involved.
- Keep the API key and signing secret private. If either leaks, make a new one
  on your payment server's **Connect** tab and paste it into the plugin on your
  WooCommerce site.

## Keep your shop patched

Your shop is the part of the system that runs PHP and WordPress. A takeover of the
shop cannot move money out of your payment server, but it could steal the API key
or show customers a wrong address. So:

- Make sure your host runs a current PHP version. Security fixes for the serious
  "PHP2Shell" bugs ([Wiz](https://www.wiz.io/blog/php2shell-php-rce)) were released
  on 21 October 2026. In WordPress: **Tools → Site Health → Info → Server** shows the
  PHP version; ask your host if it includes that update.
- Turn on automatic updates for WordPress, WooCommerce and plugins; remove plugins
  you do not use; use two-step sign-in on WordPress admin accounts.
- Never install WordPress or other PHP on the payment server.
- Optional: on your payment server's **Connect** tab, limit the API key to your shop's
  server address; and in Settings you can pause new payments instantly.
- A longer checklist for shop owners: [KEEPING_WORDPRESS_SAFE.md](KEEPING_WORDPRESS_SAFE.md).
- If the shop may have been exposed: [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md), section 6.

## Known limits

- After paying, the customer is not redirected automatically; the checkout
  page shows **Done — return to <your shop>**, which opens their WooCommerce
  order page. Order emails still go out as usual.
- Refunds are done by hand from your own wallet.
- Prices are converted at the moment the order is placed, using the Bitcoin price your
  server looks up every ten minutes (a median of five public sources). An order can
  therefore be a little cheaper or dearer than the very latest price; the customer sees
  a fixed amount in sats with a countdown.
- Your shop's currency must be one of **USD, EUR, GBP, CHF, CAD, AUD or JPY**. With
  another currency the server refuses the order (the customer is told to use another
  way to pay) until support for it is added.
- If the server cannot refresh the price for more than a day (for example it lost
  its internet connection), orders in your shop's currency are refused until it can;
  Home says so in plain words. See "Home says Bitcoin price is missing" in
  [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md).

## Translations

The plugin's settings page, checkout messages and order notes use WordPress's
normal translation system (text domain `ebp-bitcoin`). To translate, put the
`.po` / `.mo` files in the plugin's `languages` folder; `wp i18n make-pot . languages/ebp-bitcoin.pot`
makes a template. The two shop-warning texts (server not answering, orders not
being paid) are English only for now. Nothing is translated yet; there are no
language files in this version.
