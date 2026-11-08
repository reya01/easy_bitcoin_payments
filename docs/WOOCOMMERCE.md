# Connect your WooCommerce shop

Time needed: about 10 minutes. You need your payment server running with a
padlock (https) address, and admin access to your WordPress site.

**Your shop must be reachable from the internet.** The payment server tells your
shop about payments by calling its web address, and it refuses to call addresses
that only work on your own network (`localhost`, `127.0.0.1`, `192.168.…`,
`10.…`). A WordPress running only on your computer (for example "Local") will not
get payment notices; use a shop with a public address.

**Test version:** use play-money (signet) only for now. Do not take real
bitcoin yet.

## 1. Install the plugin

1. Download `ebp-bitcoin-woocommerce.zip` from the project's **Releases** page.
   Open the release you are testing (for example `v0.1.0-rc1`) and look under
   **Assets**. Do not use the "latest release" shortcut: GitHub skips
   pre-releases such as `rc1` there. Before the first release exists there is no
   zip to download; ask the project owner for one.
2. In WordPress: **Plugins → Add New → Upload Plugin**, choose the zip, then
   **Install Now** and **Activate**.
3. Go to **WooCommerce → Settings → Payments → Bitcoin (Easy Bitcoin
   Payments)** and tick **Enable**.

## 2. Fill in three things

Open your payment server's admin page and go to the **Connect** tab (its heading says "Connect your shop"). The same three steps are shown at the top of the plugin's settings page.

| In the plugin | Where to get it |
| --- | --- |
| Server address | Step 1 on the Connect tab (your padlock address) |
| API key | Step 2: **Make a new API key** (shown once, copy it right away) |
| Signing secret | Step 3: appears once when you save the notification address |

The plugin shows **Your shop's notification address**. Copy it into step 3 of
the Connect tab, press **Save address**, then copy the signing secret that
appears back into the plugin. Press **Save changes** in WooCommerce. A green
"connected to your payment server" message means it works; a red message says
what to fix.

## 3. Try it

On the Connect tab press **Send a test notification** (nothing is charged),
then place a small test order on your own shop and pay it with a signet wallet.
The order should move from **On hold** to **Processing** by itself within a few
seconds of the payment.

## What your customers see

They pick **Bitcoin** at checkout and land on your payment server's checkout
page (Ark; Lightning when your Ark server offers it; on-chain only if you
turned it on). Underpaying keeps the order on hold for a
short top-up window; if the invoice expires unpaid, the order is cancelled and
stock is released. If a payment arrives after the order was cancelled, the
order notes say so, so you can decide what to do (see **Payments that need
you** on the dashboard).

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
  on the Connect tab and paste it in.

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
- Optional: in the admin page's **Connect** tab, limit the API key to your shop's
  server address; and in Settings you can pause new payments instantly.
- If the shop may have been exposed: [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md), section 6.

## Known limits

- After paying, the customer is not redirected automatically; the checkout
  page shows **Done — return to <your shop>**, which opens their WooCommerce
  order page. Order emails still go out as usual.
- Refunds are done by hand from your own wallet.
- Prices are converted at the moment the order is placed.

## Translations

The plugin's settings page, checkout messages and order notes use WordPress's
normal translation system (text domain `ebp-bitcoin`). To translate, put the
`.po` / `.mo` files in the plugin's `languages` folder; `wp i18n make-pot . languages/ebp-bitcoin.pot`
makes a template. The two shop-warning texts (server not answering, orders not
being paid) are English only for now. Nothing is translated yet; there are no
language files in this version.
