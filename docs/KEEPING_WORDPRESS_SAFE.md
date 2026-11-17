# Keeping your WordPress shop safe

Your shop runs WordPress, WooCommerce and PHP. That is the part of the system most
people try to break into, because there are so many shops. Easy Bitcoin Payments is
built so that a hacked shop cannot move your money (see the end of this page), but
the best protection is to not get hacked. These steps take about 15 minutes once
and a few minutes a month.

## Do these once

1. **Turn on automatic updates** for WordPress, WooCommerce, your theme and your
   plugins. On the **Plugins → Installed Plugins** page, each plugin has an
   **Enable auto-updates** link. Updates are how most known holes get closed.
2. **Check your PHP version.** In WordPress: **Tools → Site Health → Info →
   Server**. Ask your hosting company if it runs the latest security update of PHP.
   If your host cannot tell you, that is a reason to look at another host. (Why this
   matters: in October 2026 the "PHP2Shell" bugs were found in PHP itself; see
   [Wiz's write-up](https://www.wiz.io/blog/php2shell-php-rce).)
3. **Delete what you do not use.** Every plugin and theme you do not use is a
   possible way in, even when switched off. Never install pirated ("nulled")
   plugins or themes; they often arrive with a back door.
4. **Protect your WordPress admin sign-in.** Use a long, unique password for every
   admin account, give admin rights to as few people as possible, and turn on
   two-step sign-in for them (a code from an app on the phone, like the one EBP
   offers). WordPress does not do this on its own; a well-known two-step plugin
   does.
5. **Use a careful host.** One that patches PHP promptly, keeps backups and only
   serves your shop over `https://`.
6. **Back up the shop itself** (orders, products, pages). This is separate from the
   backup of your payment server ([RECIPE_BACKUP.md](RECIPE_BACKUP.md)).

7. **Treat other people's scripts on your shop as a risk.** Analytics, chat,
   A/B testing, tag managers and ad tools each load JavaScript from someone else
   into your pages. If that company is hacked, its code runs in your visitors'
   browsers, and on your own domain it can read forms and change what a page says.
   This happened on 16 November 2026: attackers used a flaw at an A/B-testing
   vendor (Optimizely) to push code to 178 well-known sites, which copied card
   details, swapped crypto wallet addresses and stole cookies
   ([Help Net Security](https://www.helpnetsecurity.com/2026/11/17/optimizely-vulnerability-malicious-javascript/)).
   Keep such scripts to a minimum, remove the ones you no longer use, and ask your
   host or developer about a strict **Content Security Policy** that lists only the
   places your pages may load scripts from. Do not allow `unsafe-eval`, and
   remember that allowing a vendor's address also allows whatever that vendor
   serves. Check the shop's checkout and cart pages first, because that is where
   skimmers go.

   Your customers pay on the EBP checkout page on your payment server, not on
   your shop. That page loads no third-party scripts at all and refuses to be
   framed, so a script on your shop cannot change the amount or address shown
   there. It could still send a customer to a fake page, which is why the
   steps above matter.

## Do this now and then

- Read the WordPress admin notices and any email from the EBP plugin. The plugin
  warns you when several Bitcoin orders were placed and none was paid, or when it
  cannot reach your payment server.
- Look at your own wallet and compare it with your Bitcoin orders. The daily
  payment summary from EBP (Alerts tab, on by default once phone alerts are on)
  is a reminder to do this.

## What EBP does if the shop is taken over anyway

- The shop's API key cannot change where your money goes, change where
  notifications are sent, make a new key, resend notifications or mark payments
  as handled. Those need your admin password (and your phone code, if two-step is
  on).
- You can stop new payment requests with one switch (**Settings → Pause new
  payments**). Payments already started finish. The shop cannot undo the pause.
- Optional: limit the API key to your shop's address (on your payment server's **Connect** tab, "Extra
  protection"), so a stolen key is useless from anywhere else.

## What it cannot do

A hacked shop can still create payment requests, read a few facts (such as your
payout address) and show your customers a fake page, because it is your shop that
sends them to the payment page. If you think this happened, follow
[IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md), section 6.
