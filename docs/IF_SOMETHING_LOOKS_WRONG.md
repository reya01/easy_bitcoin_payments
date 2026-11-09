# If something looks wrong

Plain steps for a shop owner. Do the first section **first** if you think
money may be going to the wrong place; the rest can wait. Nothing here needs
the recovery words, and you should never type them anywhere while fixing a
problem.

**Test version:** with play money none of this costs anything, which makes it a
good time to practise once.

## 1. You think payments are being stolen or redirected

Signs: your own wallet received less than your shop says was paid, the
WooCommerce warning says orders stopped being paid, customers report paying an
address that is not yours, or you did not make a change you see on the
dashboard.

Do these in order. Each one is quick and none destroys anything.

1. **Pause new payments.** On the admin page go to **Settings → Pause new
   payments**. Then stop new Bitcoin orders in your shop too. In WordPress: **WooCommerce → Settings →
   Payments**, switch **Bitcoin** off. Customers can still pay another way.
   This works even if the payment server is not behaving, because it happens on
   your shop.
2. **Turn the payment server off** from your hosting company's website (use its
   power-off option; do not delete the server yet). Nothing is lost by turning
   it off, and a copy of its disk is still there for later.
3. **Look at your own wallet.** The wallet you pay out to is the one place the
   server cannot rewrite. Compare what arrived there with your paid orders.
4. **Protect the accounts around it:** change the password of your hosting
   account and turn on two-factor sign-in, and change the password of your
   WordPress admin.
5. **Write down what you saw** (times, amounts, screenshots without any secrets)
   and keep it. If you think the software itself is at fault, report it
   privately as described in `SECURITY.md`.

Only after that, decide whether to rebuild: make a **new server**, restore your
encrypted backup (see [RECIPE_BACKUP.md](RECIPE_BACKUP.md)), then create a new
API key and a new signing secret on the Connect tab and paste them into the
plugin. Do not reuse the old server.

## 2. You think someone saw your recovery words, or your admin password

- **Admin password only:** sign in and change it (Settings). If you cannot sign
  in, open the Console of your server (DigitalOcean: your Droplet, then
  **Console**), log in as `root`, and run
  `cd /opt/ebp && docker compose exec ebp ebp admin set-password`. Type the new
  password (16+ characters) and press Enter. Then download a fresh backup,
  because older backups still need the old password.
- **Lost your phone (two-step sign-in):** type one of your 10 backup codes
  instead of the 6-digit code. With none left, run
  `cd /opt/ebp && docker compose exec ebp ebp admin reset-2fa` in the server's
  Console, sign in with your password, and turn it on
  again in Settings.
- **Recovery words seen by someone else:** treat the small wallet on that
  server as no longer private. Stop using that server, set up a new one with a
  **new wallet (new words)**, and point your shop at it. The small wallet on the server is kept small
  on purpose, and your payout address (your own wallet) is not affected by
  these words.

## 3. Your API key or signing secret may have leaked

On the dashboard, **Connect** tab: **Make a new API key** (the old key stops
working immediately), and **New signing secret** (it is shown once). Paste both
into the WooCommerce plugin and press **Save changes**. Then use **Send a test
notification** to check it still works.

## 4. A customer paid but the order did not update

- Check the order's notes in WooCommerce. Notes say when a payment arrived late
  or for a different amount (in which case the order is left for you to decide).
- Check the dashboard **Payments** and **Alerts** tabs. A payment that arrived
  after the invoice expired shows under Alerts with its amount.
- If the shop and the server disagree, trust your own wallet: if the money is
  there, mark the order paid by hand.

## 5. The server is down or the page will not open

1. Wait two minutes and refresh; a restart after setup or an update takes about
   a minute.
2. Open the server's console on your hosting company's website. On a
   DigitalOcean server type `ebp-info` for the address and status.
3. The dashboard Home page lists problems in plain words, worst first, once it
   opens.
4. Known problems and fixes: [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

## 6. Your shop's website (WordPress or PHP) was hacked, or may have been exposed

Bugs in PHP or WordPress can let a stranger take over a shop without any mistake of
yours (for example [PHP2Shell](https://www.wiz.io/blog/php2shell-php-rce), fixed in
PHP updates released on 21 October 2026). Your payment server is a separate machine
and the shop cannot move money out of it, but the shop holds your **API key** and
**signing secret**, and could be made to show customers a wrong payment address.

Treat it as serious if your shop ran an unpatched PHP while open to the internet,
or if you see odd files, new admin users, unexplained orders or errors.

1. **Pause new payments** (admin page, Settings), and switch the Bitcoin payment
   method off in WooCommerce.
2. **Look at your own wallet** and compare it with your paid orders since the
   problem may have started. This is the check that cannot be faked.
3. **Make a new API key** and **New signing secret** on the **Connect** tab, and keep
   them out of the shop until the shop is clean.
4. **Clean the shop:** ask your host to restore a backup from before the problem
   and update PHP, WordPress and plugins, or rebuild it fresh. Change the passwords
   of WordPress, the hosting account and the database. Remove unknown admin users.
5. **Check the admin page's Settings** (payout address, security activity). Nothing
   there changes because of the shop, but it is a quick check that nothing else did.
6. **Reconnect** with the new key and secret ([WOOCOMMERCE.md](WOOCOMMERCE.md)),
   send a test notification, resume payments, and place a small test order.

If you cannot tell whether the shop was exposed, do steps 3 and 6 anyway; they are
quick and cost nothing.

## 7. Home says "Renewing your coins is not working"

Your coins are renewed automatically a little before they would expire. This
message means the last few renewals failed. Your money is safe for now, but coins
that are never renewed eventually expire, so do not ignore it for more than a day.

1. Look at the top of Home: is the wallet connected? If not, fix that first
   ([KNOWN_ISSUES.md](KNOWN_ISSUES.md)).
2. Wait ten minutes and refresh. The payment server's rounds can be slow, and the
   message clears on its own after a renewal succeeds.
3. Restart the server (on DigitalOcean, in the Console: `cd /opt/ebp && docker compose restart`).
4. If it is still there after a day, or it turned red ("coins are due"), move
   your money to your own wallet now: Settings, "Move everything in the till".
   Your 12 recovery words always restore the wallet.

## Practise once

On play money, do section 1 steps 1 and 2 and then restore a backup onto a new
server. Doing it once, calmly, is the best protection.
