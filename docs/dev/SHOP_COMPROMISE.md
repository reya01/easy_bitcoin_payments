# If the shop's website is taken over (planning and status)

**Why this exists.** Bugs in PHP, WordPress or a plugin can let a stranger take over
a WooCommerce site. A recent example is PHP2Shell
([Wiz write-up](https://www.wiz.io/blog/php2shell-php-rce)): two bugs in PHP itself
that, according to that write-up, allow remote code execution or memory disclosure
without logging in, on almost every PHP version of the last decade, with fixes
released on 21 October 2026. EBP is not written in PHP and runs on a separate server,
but the shop is part of the system. We cannot fix PHP. We can make a hacked shop
matter less.

**Recorded decision (8 Nov 2026):** the pause switch, owner-only key limits, optional
address lock and payment summary were added specifically in response to PHP2Shell,
so that a future PHP/WordPress takeover has a small, bounded effect. Code comments
in `src/shopkey.rs` and `src/auth.rs` point back here.

## What a hacked shop can and cannot do

| An attacker who controls the shop can... | Today |
|---|---|
| Move money out of the EBP server | **No.** Payout address, sweeps and the wallet need the admin password (and a phone code if two-step sign-in is on). A payout change waits a day. |
| Reach the EBP server, wallet or recovery words | **No**, if EBP runs on its own server (the recommended setup). Never put WordPress or any PHP on the EBP server. |
| Steal the shop's API key and signing secret | Yes (they are in the shop). |
| Use the API key to change where shop notifications go, replace the key, or mark late payments handled | **No longer.** These now need the owner's admin session. |
| Use the API key to create, read or cancel payment requests | Yes (that is the shop's job), but only up to the server's size limit per request (default 1 BTC) and 60 new requests a minute. |
| Use the API key to read the payout address, security activity, notification log or late-payment list | **No longer.** These need the owner's admin session. |
| Show customers a fake payment page or address (money goes to the attacker, never touching EBP) | **Yes.** The most dangerous case. Detected by the owner's wallet check and, if turned on, the daily payment summary. |
| Mark orders paid without payment (free goods) | Yes. EBP funds are not lost; the loss is stock. |

## What exists now

- **Pause new payments** (Settings): one switch. New payment requests are refused;
  ones already started still finish. The shop's key cannot undo it.
- **Owner-only actions:** changing the shop notification address, new signing secret,
  replacing the API key, resending notifications and marking payments handled need
  the admin session, not the shop's key. So does reading the payout address, the
  security activity, the notification log and the late-payment list.
- **Limits on the key:** one payment request can be at most `EBP_MAX_INVOICE_SATS`
  (default 100,000,000 sats = 1 BTC) and one key can make at most
  `EBP_INVOICE_RATE_LIMIT_PER_MIN` (default 60) new requests a minute; over that the
  server answers 429 "Too many payment requests".
- **Optional shop-address lock** (Connect tab): the key only works from the address
  you list, so a leaked key is useless elsewhere. Off by default. Shared hosting can
  change its outgoing address, so it is optional and easy to remove.
- **Optional payment summary** on the phone alerts: once a day (only on days with
  payments), every payment, or not at all. Counts only, never amounts. Includes a
  reminder to compare with your own wallet. Alerts themselves stay opt-in.
- **Written steps** for a hacked or exposed shop:
  [IF_SOMETHING_LOOKS_WRONG.md](../IF_SOMETHING_LOOKS_WRONG.md), section 6.

## Planned, in rough order

1. **Narrow the key further:** separate keys for "create payments" and "read status".
2. **Hourly/daily totals** on the key (the per-minute and per-request limits exist now).
3. **Daily wallet check** on the dashboard (the weekly card stays until there is real
   volume).
4. **Plugin hardening:** keep it small; no file writes; secrets in `wp-config.php`
   rather than the database; signed plugin releases.
5. **Customer-side address check** (for example [Branta](BRANTA.md)), the only
   thing that directly weakens the fake-payment-page attack.
6. **A WordPress care page:** managed hosting, automatic PHP and WordPress updates,
   few plugins, two-step sign-in on WordPress, a firewall.

## Open questions

- Is the shop's outgoing address stable enough on typical shared hosting for the
  address lock to be worth recommending?
- Should "pause" also be reachable without the dashboard (for example a link in a
  phone alert)? That would be another way in; undecided.
