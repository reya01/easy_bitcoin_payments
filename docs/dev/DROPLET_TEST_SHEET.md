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
| 8 | **Try a payment** and pay it from the wallet; it shows Paid | START_HERE step 5 | | | |
| 9 | Download a backup (Settings) | START_HERE step 7 | | | |
| 10 | Install the plugin zip in WordPress and enable it | [WOOCOMMERCE.md](../WOOCOMMERCE.md) | | | |
| 11 | Fill in server address, API key, notification address, signing secret | WOOCOMMERCE (and the steps at the top of the plugin page) | | | |
| 12 | Save: green "connected" message; **Send a test notification** | WOOCOMMERCE | | | |
| 13 | Place a small test order and pay it | WOOCOMMERCE | | | |
| 14 | Order moves to Processing by itself | WOOCOMMERCE | | | |

## Quick checks at the end

- [ ] The admin Home tab says everything is OK (or lists problems in plain words).
- [ ] The payment shows under Payments.
- [ ] The order in WooCommerce is Processing, with a note about the payment.
- [ ] Your recovery words are on paper in two places.

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
