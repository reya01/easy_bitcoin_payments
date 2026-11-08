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

1. **Stop new Bitcoin orders.** In WordPress: **WooCommerce → Settings →
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
working immediately), and make a new signing secret the same way. Paste both
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

## Practise once

On play money, do section 1 steps 1 and 2 and then restore a backup onto a new
server. Doing it once, calmly, is the best protection.
