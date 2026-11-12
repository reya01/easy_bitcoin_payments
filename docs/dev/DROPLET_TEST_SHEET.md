# Test-day sheet: DigitalOcean + WooCommerce (play money only)

Print it or keep it open beside you. Do the steps in order and **write down
every place you hesitated, guessed or got stuck**, even if you got past it. Those
notes are the result of the test. Times are guesses; write the real ones.

Play money only: signet coins. Do not send real bitcoin.

**Round 2 (after the 8 November test).** Round 1 proved the install, the wizard,
Lightning and Ark payments, pause and backup. Round 2 repeats that quick path (time
it again) and then covers what changed since and what round 1 could not reach: the
WooCommerce shop, the new checkout, dollar amounts, the Home balance, the backup
check, and a day of running unattended. The new things are in
**"New since round 1"** below.

## Before you start (about 30 minutes, can be done a day earlier)

- [ ] A signet wallet that can receive coins and pay (the phone wallets are
      mainnet only, so use the bark command-line wallet on your computer:
      <https://second.tech/docs/getting-started/bark-cli/signet>), with some
      free play coins from the faucet (<https://signet.2nd.dev/>, needs a GitHub
      login). They take 5 to 10 minutes to arrive. When you paste a payment code
      into a terminal, put it in single quotes (it contains `&` and `?`).
- [ ] A password manager open, and pen and paper (for the 12 recovery words).
- [ ] A WordPress + WooCommerce shop with a **public** address (not
      "localhost" or a site only on your own computer), where you are an admin.
- [ ] The plugin zip `ebp-bitcoin-woocommerce.zip`, downloaded from the
      [Releases page](https://github.com/reya01/easy_bitcoin_payments/releases) (top release, under **Assets**).
- [ ] Your shop's currency is one of USD, EUR, GBP, CHF, CAD, AUD or JPY (other
      currencies are refused for now).
- [ ] Your shop's PHP is up to date (WordPress: Tools, Site Health, Info, Server;
      PHP security fixes were released 21 October 2026, see
      [WOOCOMMERCE.md](../WOOCOMMERCE.md#keep-your-shop-patched)). Play money only,
      but check anyway so the habit is there.
- [ ] Your DigitalOcean account with a payment method added.
- [ ] The setup text copied (START_HERE.md step 1).

## The steps

| # | Step | Where it is explained | Started | Done | Notes: what surprised or confused you |
|---|------|------------------------|---------|------|----------------------------------------|
| 1 | Create the Droplet (Ubuntu 24.04, 2 GB, paste the setup text) | [START_HERE.md](../START_HERE.md) steps 1-2 | | | |
| 2 | Wait 15 to 30 minutes | START_HERE step 3 | | | |
| 3 | Open the Console and read the address and SETUP CODE | START_HERE step 3 | | | |
| 4 | Open the address; check there is a padlock | START_HERE step 3 | | | |
| 5 | Setup wizard: code, wallet, **write down 12 words**, quiz | START_HERE step 4 | | | |
| 6 | Store name, admin password (16+ characters), payout address | START_HERE step 4 | | | |
| 7 | Save the API key; sign in at `/admin` | START_HERE step 4 | | | |
| 7b | On first sign-in, turn on two-step sign-in with an authenticator app; save the 10 backup codes; sign out and back in with a code | START_HERE step 4 | | | |
| 8 | **Try a payment**; the checkout page shows a Lightning invoice first, with the dollar value next to the sats. Pay it from the wallet; it shows Paid | START_HERE step 5 | | | |
| 9 | Download a backup (Settings), then **Check a backup** with the file you just saved | START_HERE step 7 | | | |
| 10 | Install the plugin zip in WordPress and enable it | [WOOCOMMERCE.md](../WOOCOMMERCE.md) | | | |
| 11 | Fill in server address, API key, notification address, signing secret | WOOCOMMERCE (and the steps at the top of the plugin page) | | | |
| 12 | Save: green "connected" message; **Send a test notification** | WOOCOMMERCE | | | |
| 13 | Place a small test order and pay it | WOOCOMMERCE | | | |
| 14 | Order moves to Processing by itself | WOOCOMMERCE | | | |

## Money safety (do these first)

These are the most important checks in this round: they prove money reaches **your
own wallet**. No earlier round has tested this.

- [ ] **Move money out by hand.** On your payment server's Home, press **Move my
      money to my payout address now**. Check the money arrives in the wallet whose
      address you gave as the payout address, and that it shows under **Recent
      transfers to your own wallet**. Write down how long it took and the amount
      that arrived compared to what Home showed (any fee).
- [ ] **Change how much the server keeps (Settings).** In Settings, under **Where your
      money goes**, find **How much to keep on this server**. It should show the current
      amount. Type a new amount (for example 2000), enter your admin password (and a
      2FA code if asked) and save: it should say saved and show the new amount straight
      away. Then pay a test payment above it and check the next transfer leaves exactly
      that much on the server. Press **Go back to the default** and check it shows
      "(the default)" again.
- [ ] **Money moves out by itself.** On your payment server's Home, use the test
      payment box to make one test payment of **2000** sats and pay it (this passes
      the 1,000-sat limit the server keeps). Within about 10 minutes, without
      pressing anything, a transfer to your payout wallet should appear under
      **Recent transfers to your own wallet**, and the money should arrive there.
- [ ] **Restore drill (new server).** Follow `docs/RECIPE_BACKUP.md` → "Practice a restore once": on your current server note the balance, orders count and payout address, download a backup, **power off the first droplet** (never run both at once: same wallet, same shop connection), then make a second server and restore it from the 12 words and the backup file on its setup page. Write down anything that did not match the page, and whether the balance, orders, payout address and shop connection all came back. Also check, in the first hour after the restore, that no old order suddenly shows as overpaid and no "payment can't be matched" item appears (the restored wallet re-reads its coins; this is the part automated tests cannot cover).
- [ ] **Which build is running.** On your payment server's Home, the **Version** line
      says `0.1.0-rc1`. (After any later update, check it changed.)
- [ ] **Restoring from the 12 words:** not testable yet. There is no restore screen or
      written restore steps yet; see the release checklist. Keep your words safe anyway.

## New since round 1 (please check each one)

- [ ] **Lightning is there at all.** Do this one first: create a test payment and check the checkout page opens on Lightning. If it opens on "One code, any wallet" instead, Lightning invoices are failing: write it down and send the last lines of the server log (`cd /opt/ebp && docker compose logs --tail 50 ebp`), with nothing secret in them.
- [ ] **Lightning first.** The checkout page opens on a Lightning invoice, says "Paying with Lightning" and shows the small network fee in plain view.
- [ ] **Other ways to pay.** Under it: "One code, any wallet", Ark, and Regular Bitcoin. Pick each one: the shown code changes, the **Copy** button copies exactly what is shown, and Regular Bitcoin shows just an address (starting `tb1`), not a long `bitcoin:` code. Paste any `bitcoin:` code into your terminal inside single quotes.
- [ ] **Dollars next to sats.** On the checkout page, Home, Payments, the test-payment box and the payout/transfer lists you see things like "1,000 sats ($0.40)". The dollar figure is roughly right for today's real Bitcoin price (play coins are worth nothing, but the number should look sensible). Write down anywhere sats show with **no** dollar figure.
- [ ] **Home's balance.** Pay two or three small test payments. "Held on your server" counts money that is still arriving, with a grey note saying so, instead of showing 0 while coins refresh. Routine upkeep (renewals) is in its own **Routine upkeep** card, not mixed into the money-moved list.
- [ ] **Bitcoin price line.** Right after the server first starts, Home may say "Bitcoin price not available yet". It should disappear within a minute or two by itself and stay away. If it stays, write down the exact Home text (the server cannot reach the price websites).
- [ ] **Check a backup.** Settings → Check a backup works with the downloaded file and the right password, and says clearly when the password is wrong.
- [ ] **Leave it running for a day.** After the tests, leave the server on. The next day, open Home: copy its text, and on the server run `cd /opt/ebp && ./scripts/drill_capture.sh` and save the file it writes. We want to see that coins keep being renewed quietly and "Held on your server" stays sensible, with no red items.
- [ ] **Lightning fee.** On a Lightning checkout, if a fee note shows, it says what your wallet will show (order plus fee); pay it and check the order shows Paid, not "part of your payment". Write down the amounts your wallet showed.
- [ ] **No false coin alerts.** With phone alerts on, leave the server a day: there should be no "Some coins are close to expiring" alert while Home is green.
- [ ] **Payout change alert.** With phone alerts on, start a payout address change in Settings (any valid test address), check your phone gets "Your payout address is being changed" within a minute, then cancel the change.
- [ ] **A lost notice still finishes the order.** (Optional, after step 14.) On your
      payment server's Connect tab, note the webhook address in card 3, then change
      one letter in it and press Save address. Place and pay a small test order: it
      stays On hold. Within about an hour it should move to Processing by itself, with
      a note "Found by the hourly check". Then put the correct address back (copy it
      from the plugin on your WooCommerce site), press Save address, paste the new
      signing secret into the plugin and press Save changes.
- [ ] **A shop order priced in your currency.** (WooCommerce, step 13.) The amount in sats on the checkout page matches your shop total at roughly the current Bitcoin price.

## Extra checks only a real server can answer (if time allows)

CI already covers these on a fake wallet and a local shop. These need the real thing:

- [ ] **The real address reaches EBP.** On the Connect tab, open "Extra protection" and save a wrong address (for example `203.0.113.7`). Place a small test order from the shop. Checkout should fail, and the Connect tab should then say a request with your shop key from **your shop's real public address** was refused. If it shows a `172.x` or `10.x` address instead, the proxy is hiding the real one: write it down. Then clear the box and Save so checkout works again.
- [ ] **Pause works from the shop's side.** Settings → Pause new payments. At the shop's checkout, choosing Bitcoin should say it is not available and offer another method. Resume and confirm it works again.
- [ ] **Phone alerts and the daily summary.** Turn on phone alerts, send a test alert, and look at the "Tell me about payments" choice (it should say "Once a day"). Pay one more test order and check whether the daily message arrives the next day, with a count and no amounts.
- [ ] **A real phone scan.** Scan the checkout QR with your signet wallet and with the camera. Also tap **Open in my wallet app** on the phone.
- [ ] **Restore turns two-step off.** (Optional, only if you have a spare server.) After restoring a backup, sign in with the password only; Home should offer two-step sign-in again.
- [ ] **Owner-only pages still load.** Signed in at `/admin`, check that the payout address (Settings), security activity, notification log and (after the expiry test below) the late-payment list all show up. Those pages now need your admin sign-in, not the shop's key; WooCommerce checkout must keep working throughout.
- [ ] **A cancelled or expired payment page.** Let a test payment expire and open its page: it should say it expired, hide the QR, and offer a way back to the shop.

## Quick checks at the end

- [ ] The admin Home tab says everything is OK (or lists problems in plain words).
- [ ] The payment shows under Payments.
- [ ] The order in WooCommerce is Processing, with a note about the payment.
- [ ] Your recovery words are on paper in two places.

## Likely snags and what to try

| What you see | Try this |
|--------------|----------|
| The Console shows only `login:` after 30+ minutes | Log in as `root`, run `cloud-init status`, then `tail -n 40 /var/log/ebp-vps-setup.log`. "running" means still installing. |
| A box saying "No setup code yet" or "still starting" | Wait 2 minutes, log in, run `ebp-info`. |
| `ebp-info`: command not found | The install has not reached its last step. Wait, then run `cd /opt/ebp && ./scripts/vps-setup.sh`. |
| The address does not open | Wait 5 minutes. Then check ports 80 and 443 are open (a DigitalOcean Firewall, if you made one). |
| Browser warns the certificate is wrong | Wait a few minutes (the padlock is still being issued), then reload. Do not click through a warning. |
| Wizard says the setup code is wrong | Type the code again (capitals, dashes and spaces do not matter). If the server restarted, run `ebp-info` for the new code. |
| The page does not answer right after "Finish" | The server restarts once; wait about a minute, then open `/admin`. |
| Forgot the admin password | In the Console: `cd /opt/ebp && docker compose exec ebp ebp admin set-password` |
| Lost your phone, no backup codes | In the Console: `cd /opt/ebp && docker compose exec ebp ebp admin reset-2fa` |
| "Connected" never appears in the WooCommerce plugin | The shop address must be public (not localhost). Re-check the API key and the https address. |
| Test notification fails | The shop must be reachable from the internet; see WOOCOMMERCE.md. |
| Your bark wallet says "Invalid payment request / No valid payment option found" for the "One code, any wallet" code | Put the whole code in single quotes: `bark send 'bitcoin:?amount=…&lightning=…&ark=…'`. Without quotes the terminal cuts the code at the first `&`. |
| Home says "Bitcoin price not available yet" or "out of date" | Wait two minutes and reload. If it stays, the server may have no internet: in the Console run `curl -sS https://mempool.space/api/v1/prices` and write down what it prints. Dollar amounts disappear meanwhile; payments still work. |
| A shop order fails with "Bitcoin price ... is not available right now" | The server has no recent price. Wait a few minutes and try again; see "Home says Bitcoin price ..." in IF_SOMETHING_LOOKS_WRONG.md. |
| The test payment never shows Paid | Check the wallet really sent play coins (signet, not mainnet) and that Home says the wallet is connected. |

## If something goes wrong

1. Stop and write down exactly what you saw (a screenshot is best).
2. Look in [KNOWN_ISSUES.md](../KNOWN_ISSUES.md) and the "If something goes wrong"
   part of [START_HERE.md](../START_HERE.md).
3. To report it (the "Something went wrong" form on GitHub Issues lists what to include), collect: the text of the Console box, the **Home** tab text, and
   the last lines of the install log (in the Console:
   `tail -n 50 /var/log/ebp-vps-setup.log`).

**Never share, anywhere:** the 12 recovery words, your admin password, the API
key or the signing secret. Anyone asking for them is not helping you.

## Overall, afterwards

- Total time from "Create Droplet" to the first paid test order:
- The one step that was hardest:
- Would you have finished without help? Yes / No. If no, where did you need it:
- Anything the written instructions got wrong or left out:
