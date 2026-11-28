# A whole shop on DigitalOcean (advanced option, first draft)

> **Status: first draft, in testing, for more advanced users.** It has not been
> run end to end on a real DigitalOcean droplet yet. Use play money (signet)
> while trying it.
>
> **Most people should not use this page.** The recommended way is a hosted
> WooCommerce shop that someone else looks after (for example WooCommerce.com
> hosting), connected to your payment server with
> [WOOCOMMERCE.md](WOOCOMMERCE.md). A hosted shop patches PHP, backs itself up
> and sends email for you. Use this page if you want to run the shop yourself,
> on the same host as your payment server.

This recipe takes you from an empty DigitalOcean account to:

- a **WooCommerce shop** on one droplet, and
- an **Easy Bitcoin Payments server** on a second droplet,
- connected to each other, with a paid test order.

You can follow it by hand, or give it to an AI assistant and let it walk you
through ([below](#using-an-ai-assistant)).

**Why two droplets?** If someone breaks into the shop (WordPress is the part
people attack most), they must not be able to reach the payment server. On
separate droplets they cannot. Never put both on one droplet. See
[docs/dev/SHOP_COMPROMISE.md](dev/SHOP_COMPROMISE.md).

**What you are taking on.** Running your own shop means you apply its updates,
back it up, and keep it safe ([KEEPING_WORDPRESS_SAFE.md](KEEPING_WORDPRESS_SAFE.md)).
Plan on 15 minutes a month, and more on a bad day.

## Places you will use

Every step says which one it means:

- **DigitalOcean**: the DigitalOcean website (control panel).
- **Shop console**: the shop droplet's **Console** button in DigitalOcean (a
  black text window in your browser).
- **Shop admin**: your shop's WordPress admin, at your shop address followed by `/wp-admin`.
- **Payment server**: the Easy Bitcoin Payments admin, at its address followed by `/admin`.
- **Phone wallet**: Noah (Android) or Arké (iPhone).

## Before you start

- A DigitalOcean account with **two-step sign-in turned on**.
- A **password manager**. You will save several passwords and keys.
- Optional but best: your own domain name for the shop (for example
  `myshop.com`). Have it ready before you start, because moving a WordPress shop
  to a new name later is extra work.

## 1. The payment server

**DigitalOcean, payment server:** follow [START_HERE.md](START_HERE.md) to the end
of setup. Save the **API key** in your password manager. Stay on play money.

## 2. The shop droplet: DigitalOcean's WordPress 1-Click

DigitalOcean makes a ready-made **WordPress** droplet, kept up to date by
DigitalOcean (Ubuntu 24.04, Caddy for the padlock, PHP 8.3, MySQL, WP-CLI, a
firewall and fail2ban). We build on that rather than on our own setup.

1. **DigitalOcean:** Create → Droplets → **Marketplace**, choose **WordPress**.
   Use a **2 GB** memory size or more (DigitalOcean's minimum is 1 GB; 2 GB is
   its advice for a real shop) and the same region as your payment server.
2. **DigitalOcean:** under authentication choose a password or an SSH key, then
   create it. Wait a few minutes.
3. **Shop console:** open the droplet's **Console**. On the first login a setup
   script asks you for an **email address, an admin username, an admin password
   and a site title**. Type them yourself, with a long password from your
   password manager. (This is the one step that cannot be done from a startup
   box, which is why this is an advanced route.)
4. Your shop is now at its IP address with a padlock. Before the setup finishes,
   the address only shows a "setup pending" page.
5. **Own domain (recommended):** **at your domain company**, add an **A record**
   for the shop name pointing at the droplet's IP, wait a few minutes, then in the
   **Shop console** run `/root/wp_setup_domain.sh` and follow it. It switches the
   padlock to your name. Do this **before** you add products.

The Marketplace page for the image is the authority on these steps; if what you
see differs from this page, follow the Marketplace page and tell us.

## 3. Add WooCommerce and the Bitcoin plugin

**Shop admin:**

1. **Plugins → Add New**, search **WooCommerce** (by Automattic), **Install
   Now**, **Activate**. Answer WooCommerce's own setup questions (store address,
   what you sell). Add one test product priced under one dollar.
2. Turn on two-step sign-in and automatic updates
   ([KEEPING_WORDPRESS_SAFE.md](KEEPING_WORDPRESS_SAFE.md)).
3. Install our plugin as in section 1 of [WOOCOMMERCE.md](WOOCOMMERCE.md):
   download `ebp-bitcoin-woocommerce.zip` from the
   [Releases page](https://github.com/reya01/easy_bitcoin_payments/releases),
   then **Plugins → Add New → Upload Plugin**.

New WooCommerce shops start in **Coming soon** mode: visitors see a placeholder
until you press **Launch your store** (WooCommerce → Settings → Site
visibility). That is fine while you test.

## 4. Connect the shop to the payment server

Do **sections 2 and 3** of [WOOCOMMERCE.md](WOOCOMMERCE.md): server address, API
key, signing secret, then a test order paid from your phone wallet. The order
should move from **On hold** to **Processing** by itself.

## Using an AI assistant

Copy the text below into an AI assistant (Claude, or another), with the link to
this page.

> **Never give the assistant your passwords, API key, signing secret, recovery
> words or two-step codes.** You type those yourself, including the admin
> password the setup script asks for. If you share a screenshot, cover them first.

```
Please walk me through this recipe, one step at a time:
https://github.com/reya01/easy_bitcoin_payments/blob/main/docs/SHOP_ON_DIGITALOCEAN.md

Rules for you:
- Follow the recipe and the pages it links to. Do not invent other steps on the
  payment server.
- For every step, say where it happens: DigitalOcean, shop console, shop admin,
  payment server, or phone wallet.
- Give me one step, then wait until I say it is done or describe what I see.
- Never ask me for passwords, the API key, the signing secret, recovery words
  or two-step codes. If I start to paste one, tell me to stop and change it.
- Keep the shop and the payment server on two separate droplets. Never suggest
  combining them.
- Stay on play money (signet) until the test order has worked.
- If something does not match the recipe, stop and ask me to describe what I
  see. If a payment seems missing, stop and point me to
  docs/IF_SOMETHING_LOOKS_WRONG.md instead of trying fixes.
```

## Other ways to set up the shop droplet

- **Our own startup script (more advanced still).** `deploy/shop-cloud-init.yaml`
  and `scripts/shop-setup.sh` build a similar shop (Caddy, PHP 8.3, MariaDB,
  WordPress, WooCommerce, our plugin after its signature check, firewall,
  automatic security updates) from the startup box, with no questions asked at
  first login; the admin password is written to `/root/shop-info.txt`. Its
  advantage is that CI runs it on a clean Ubuntu 24.04 on every change; the
  1-Click image cannot be tested that way. Its disadvantage is that we maintain
  it. Paste the file into **Advanced options → Add Initialization scripts** when
  you create a plain Ubuntu 24.04 droplet, then read the details with
  `sudo cat /root/shop-info.txt` in the Shop console.
- **DigitalOcean's "WordPress + WooCommerce" 1-Click.** Made by a third party
  (the Marketplace page does not name them), with Apache and the same kind of
  first-login questions. We do not recommend it for a shop that takes payments.

## Not done yet (known gaps)

- **Never run end to end on a real droplet yet.** Next step.
- **Shop emails.** DigitalOcean blocks the email ports (25, 465 and 587) on
  droplets, so WooCommerce cannot send order emails on its own. You need an
  email-sending service and a plugin for it. Not covered yet.
- **Scheduled jobs.** WordPress runs its timed jobs when someone visits. Our
  plugin's hourly check of unpaid orders runs from the same timer, so a quiet
  shop checks late. Our own script sets up a real timer; with the 1-Click you
  can do the same (see the WordPress documentation on `DISABLE_WP_CRON`), not
  yet written up here.
- **Shop backups.** DigitalOcean's droplet backups (a paid option) or a
  WordPress backup plugin. The payment server has its own backup
  ([RECIPE_BACKUP.md](RECIPE_BACKUP.md)).
- **Shop address lock** on the payment server (only accept calls from the
  shop droplet's IP): to be added to step 4.
- **The two droplets talk over the public internet** with https. A private
  network (DigitalOcean VPC) would need the payment server to accept a private
  address for notices, which it refuses on purpose today.
- Tried with more than one AI assistant, and by someone who did not write it
  (the 0.9 milestone).
