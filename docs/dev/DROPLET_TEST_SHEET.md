# Test-day sheet: DigitalOcean + WooCommerce (play money only)

Print it or keep it open beside you. Do the steps in order and **write down
every place you hesitated, guessed or got stuck**, even if you got past it. Those
notes are the result of the test. Times are guesses; write the real ones.

Play money only: signet coins. Do not send real bitcoin.

## Before you start (about 30 minutes, can be done a day earlier)

- [ ] A signet wallet that can receive coins (the tested one is **Arké**,
      iPhone), with some free play coins from the faucet
      (<https://signet.2nd.dev/>, needs a GitHub login). They take 5 to 10 minutes
      to arrive.
- [ ] A password manager open, and pen and paper (for the 12 recovery words).
- [ ] A WordPress + WooCommerce shop with a **public** address (not
      "localhost" or a site only on your own computer), where you are an admin.
- [ ] The plugin zip `ebp-bitcoin-woocommerce.zip` saved where your browser
      can reach it.
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
| 8 | **Try a payment** and pay it from the wallet; it shows Paid | START_HERE step 5 | | | |
| 9 | Download a backup (Settings) | START_HERE step 7 | | | |
| 10 | Install the plugin zip in WordPress and enable it | [WOOCOMMERCE.md](../WOOCOMMERCE.md) | | | |
| 11 | Fill in server address, API key, notification address, signing secret | WOOCOMMERCE (and the steps at the top of the plugin page) | | | |
| 12 | Save: green "connected" message; **Send a test notification** | WOOCOMMERCE | | | |
| 13 | Place a small test order and pay it | WOOCOMMERCE | | | |
| 14 | Order moves to Processing by itself | WOOCOMMERCE | | | |

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
