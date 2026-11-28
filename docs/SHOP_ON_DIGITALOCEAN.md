# A whole shop on DigitalOcean (recipe, first draft)

> **Status: first draft, in testing.** The roadmap goal is that this recipe is
> ready at or near 1.0. The shop script is tested automatically on a clean
> Ubuntu 24.04 machine on every change, but **it has not been run on a real
> DigitalOcean droplet yet.** Use play money (signet) while trying it.

This recipe takes you from an empty DigitalOcean account to:

- a **WooCommerce shop** on one droplet, and
- an **Easy Bitcoin Payments server** on a second droplet,
- connected to each other, with a paid test order.

You can follow it by hand, or give it to an AI assistant and let it walk you
through (see [Using an AI assistant](#using-an-ai-assistant)).

**Why two droplets?** If someone breaks into the shop (WordPress is the part
people attack most), they must not be able to reach the payment server. On
separate droplets they cannot. Never put both on one droplet. See
[docs/dev/SHOP_COMPROMISE.md](dev/SHOP_COMPROMISE.md).

## Places you will use

Every step says which one it means:

- **DigitalOcean**: the DigitalOcean website (control panel).
- **Shop console**: the shop droplet's **Console** button in DigitalOcean (a
  black text window in your browser). Only needed once, to read the sign-in details.
- **Shop admin**: your shop's WordPress admin, at your shop address followed by `/wp-admin`.
- **Payment server**: the Easy Bitcoin Payments admin, at its address followed by `/admin`.
- **Phone wallet**: Noah (Android) or Arké (iPhone).

## Before you start

- A DigitalOcean account with **two-step sign-in turned on**.
- A **password manager**. You will save several passwords and keys.
- Optional: your own domain name for the shop (for example `myshop.com`). You
  can start on a free address and add your own later, but moving a WordPress
  shop to a new name later is extra work, so if you have one, use it from the start.

## 1. The payment server

**DigitalOcean, payment server:** follow [START_HERE.md](START_HERE.md) to the end
of setup. Save the **API key** in your password manager. Leave it on play money.

## 2. The shop droplet

1. Optional, own domain: **at your domain company**, add an **A record** for the
   shop name pointing at the droplet's IP. You only know the IP after step 2, so
   you can also create the droplet first, add the record, wait a few minutes,
   and then do step 3 with your name filled in on a new droplet. (Making this
   smoother is on the list below.)
2. **DigitalOcean:** Create → Droplets. Choose **Ubuntu 24.04**, a **2 GB**
   memory size (1 GB works for trying it out; the script adds a swap file), the
   same region as your payment server.
3. Under **Advanced options**, tick **Add Initialization scripts (free)** and
   paste the whole of [`deploy/shop-cloud-init.yaml`](../deploy/shop-cloud-init.yaml).
   Change `SHOP_TITLE` and, if you have one ready, `SHOP_DOMAIN`. Leave
   `SHOP_DOMAIN=` empty to start on a free `…sslip.io` address.
4. Create it and wait about 10 minutes.
5. **Shop console:** open the droplet's **Console** and type:

   ```
   sudo cat /root/shop-info.txt
   ```

   It shows the shop address, the WordPress admin address, the username and a
   long random password. Save them in your password manager. If the file is not
   there yet, wait a few more minutes; progress is in `/var/log/shop-setup.log`.

What the script did for you: Caddy (web server with an automatic padlock), PHP 8.3,
MariaDB, WordPress, WooCommerce, the Easy Bitcoin Payments plugin (installed only
after its release signature checked out, not yet connected), a firewall that
allows only ports 22, 80 and 443, fail2ban, automatic security updates, and a
real timer for WordPress's scheduled jobs. It blocks a few addresses shops do not
need and that attackers like (`xmlrpc.php`, PHP files in uploads).

## 3. Make the shop yours

**Shop admin:**

1. Sign in with the details from step 2, then change the password
   (**Users → Profile**) to one from your password manager.
2. **Settings → General:** put your real email address.
3. Turn on two-step sign-in and automatic updates
   ([KEEPING_WORDPRESS_SAFE.md](KEEPING_WORDPRESS_SAFE.md)).
4. **Shop console:** delete the file with the first password: `sudo rm /root/shop-info.txt`.
5. **WooCommerce → Home:** answer WooCommerce's own setup questions (store
   address, what you sell). Add one test product priced under one dollar.

New WooCommerce shops start in **Coming soon** mode: visitors see a placeholder
until you press **Launch your store** (WooCommerce → Settings → Site visibility).
That is fine while you test.

## 4. Connect the shop to the payment server

The plugin is already installed, so skip section 1 of
[WOOCOMMERCE.md](WOOCOMMERCE.md) and do **sections 2 and 3**: server address, API
key, signing secret, then a test order paid from your phone wallet. The order
should move from **On hold** to **Processing** by itself.

## Using an AI assistant

Copy the text below into an AI assistant (Claude, or another), with the link to
this page.

> **Never give the assistant your passwords, API key, signing secret, recovery
> words, two-step codes or the contents of `shop-info.txt`.** You type those
> yourself. If you share a screenshot, cover them first.

```
Please walk me through this recipe, one step at a time:
https://github.com/reya01/easy_bitcoin_payments/blob/main/docs/SHOP_ON_DIGITALOCEAN.md

Rules for you:
- Follow the recipe and the pages it links to. Do not invent other steps on the
  payment server.
- For every step, say where it happens: DigitalOcean, shop console, shop admin,
  payment server, or phone wallet.
- Give me one step, then wait until I say it is done or describe what I see.
- Never ask me for passwords, the API key, the signing secret, recovery words,
  two-step codes or the contents of shop-info.txt. If I start to paste one,
  tell me to stop and change it.
- Keep the shop and the payment server on two separate droplets. Never suggest
  combining them.
- Stay on play money (signet) until the test order has worked.
- If something does not match the recipe, stop and ask me to describe what I
  see. If a payment seems missing, stop and point me to
  docs/IF_SOMETHING_LOOKS_WRONG.md instead of trying fixes.
```

## Another way: DigitalOcean's WordPress 1-Click

DigitalOcean offers a ready-made **WordPress** droplet (Ubuntu 24.04, Caddy,
PHP 8.3, MySQL, WP-CLI), maintained by DigitalOcean. It cannot be finished
from a startup script: on the first **Console** login it asks for an email,
username, password and site title. After that, install WooCommerce from
**Plugins → Add New** and upload the Easy Bitcoin Payments plugin zip as in
[WOOCOMMERCE.md](WOOCOMMERCE.md). That route works too; this recipe uses its own
script so the whole shop can be set up from the startup box and tested
automatically.

There is also a third-party **WordPress + WooCommerce** 1-Click in the
Marketplace (Apache, an interactive setup on first login). It is not maintained
by DigitalOcean or by us.

## Not done yet (known gaps)

- **Never run on a real droplet yet.** Next step.
- **Shop emails.** DigitalOcean blocks the email ports (25, 465 and 587) on
  droplets, so WooCommerce cannot send order emails on its own. You need an
  email-sending service and a plugin for it. Not covered yet.
- **Own domain on a new droplet:** the IP is only known after the droplet
  exists. A smoother path (a Reserved IP first, or a "change the shop's address"
  helper) is to be designed.
- **Shop backups.** DigitalOcean's droplet backups (a paid option) or a WordPress
  backup plugin. Not covered yet. The payment server has its own backup
  ([RECIPE_BACKUP.md](RECIPE_BACKUP.md)).
- **Shop address lock** on the payment server (only accept calls from the
  shop droplet's IP): to be added to step 4.
- **The two droplets talk over the public internet** with https. A private
  network (DigitalOcean VPC) would need the payment server to accept a private
  address for notices, which it refuses on purpose today.
- Tried with more than one AI assistant, and by someone who did not write it
  (the 0.9 milestone).
