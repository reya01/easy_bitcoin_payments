=== Easy Bitcoin Payments for WooCommerce ===
Tags: bitcoin, payments, woocommerce, ark, lightning
Requires at least: 6.2
Requires PHP: 7.4
Stable tag: 0.1.0-rc2
License: MIT
License URI: https://opensource.org/license/mit

Accept Bitcoin in WooCommerce through your own Easy Bitcoin Payments server. Experimental test version.

== Description ==

**This is an experimental test version.** Use a payment server in test mode (play-money signet coins). A real-money payment server is possible but not yet tried with real bitcoin: keep amounts tiny.

This plugin adds a "Bitcoin" payment method to WooCommerce. It does not handle any money itself. It talks to **your own** Easy Bitcoin Payments server, which holds a small wallet that is emptied to a wallet you control.

What it does:

* At checkout, creates an invoice on your payment server and sends the customer to its hosted checkout page.
* When the payment server says an invoice was paid, **asks the server again directly** before it marks the order paid (a forged notification cannot mark an order paid).
* Marks the order paid only when the full amount arrived; keeps it on hold for a partial payment; cancels it when the invoice expires.
* Refuses to auto-complete an order whose total changed after the invoice was made.
* Shows up in the block-based checkout (covered by the automated test) and in the classic checkout through WooCommerce's standard payment method interface (not separately tested). Declares compatibility with high-performance order storage.
* Warns you if the payment server stops answering or Bitcoin orders stop getting paid (a red notice in the admin and one email a day per problem). The check runs from your shop, so it still works when the payment server is down.
* Removes its saved settings, including the API key and signing secret, when you delete the plugin.

It is **not** included: refunds (Bitcoin payments are final; refund by hand), Shopify, or hosting. You run the payment server yourself.

Setup guide: see `docs/WOOCOMMERCE.md` in the project repository.

== External services ==

The plugin connects only to the payment server address you enter in its settings (your own server). It does not contact the plugin author or any other service.

Data sent to your payment server:

* When a customer chooses Bitcoin: the order number, the order total and currency, the word "woocommerce", and the "thank you" page address of the order (so the checkout page can send the customer back).
* When you save settings: a request that asks whether the server is up, and one that checks the API key is accepted.
* Once an hour, from WordPress cron: a request that asks whether the server is up (used for the warning described above).
* After a payment notice arrives: a request that asks the server for that invoice's status.

Stored in your WordPress database (in the plugin's settings): the server address, the API key and the signing secret. Delete the plugin to remove them.

== Installation ==

1. Upload the plugin zip (Plugins > Add New > Upload Plugin) and activate it.
2. Go to WooCommerce > Settings > Payments > Bitcoin (Easy Bitcoin Payments) and tick Enable.
3. Fill in the server address, API key and signing secret from your payment server's admin page (a different website from your shop: the Connect tab, at the bottom of the screen). Paste the "notification address" shown here into that tab.
4. Press Save changes. A green message says the connection works; a red one says what to fix.
5. Place a small test order (with a signet wallet on a play-money server).

Your server needs a padlock (https) address.

== Frequently Asked Questions ==

= Where does the money go? =

To a small wallet on your own payment server, which is emptied to the payout address you choose there. This plugin never sees or moves money.

= Can I use it for real money? =

Not yet. This is a test version. See the project repository for the checklist before a real release.

= Why does a paid order stay "On hold"? =

Usually the payment server cannot reach your shop (wrong notification address, or your shop blocks outside requests), or the signing secret does not match. The settings page and the payment server dashboard both say what to check. See `docs/WOOCOMMERCE.md`.

= Are the plugin's messages translatable? =

Most messages shown to customers and in the settings page are. The text of the two tripwire warnings (server not answering, orders not being paid) is English only for now. Put translation files in the plugin's `languages` folder.

= Which WordPress and WooCommerce versions are tested? =

An automated test installs the latest WordPress and WooCommerce and runs the checkout and payment flows on every change to the project. No older versions are tested. The plugin declares WooCommerce 8.0 and WordPress 6.2 as minimums, which are not individually tested.

== Changelog ==

= 0.1.0 =
* First test version.

== Upgrade Notice ==

= 0.1.0 =
First test version. Play money only.
