# Start here

Easy Bitcoin Payments (EBP) lets your WooCommerce shop take Bitcoin into a
wallet **you** control, with no company in the middle. This page takes you from
nothing to a first paid test order. It uses a small DigitalOcean server, and you
do not need a terminal.

**This is a test version: play money (signet) only.** Do not use real bitcoin.

**You need**

- A DigitalOcean account with a payment method (the server costs about $12 a month).
- A WooCommerce shop with a **public** web address, where you are an admin.
- A wallet for the payout address. The tested ones: **Arké** or **Noah** (Ark),
  **Sparrow**, **Nunchuk** or **Liana** (on-chain). Use a signet wallet for play money.
- Free play coins from <https://signet.2nd.dev/> (needs a GitHub login; they take
  5 to 10 minutes to arrive).
- Pen and paper (for 12 recovery words) and a password manager.
- About an hour, mostly waiting.

## 1. Copy the setup text

DigitalOcean will ask for a block of setup text. Copy it first.

Open <https://github.com/reya01/easy_bitcoin_payments/blob/main/deploy/cloud-init.yaml>
and tap the **copy icon** at the top right of the file. (Same file, without its
comments, shown here in case you prefer to copy it from this page.)

```
#cloud-config
package_update: true
package_upgrade: true
packages:
  - git
  - curl
  - ca-certificates
  - docker.io
  - docker-compose-v2
runcmd:
  - [ sh, -c, "git clone --depth 1 --branch main https://github.com/reya01/easy_bitcoin_payments.git /opt/ebp" ]
  - [ sh, -c, "/opt/ebp/scripts/vps-setup.sh > /var/log/ebp-vps-setup.log 2>&1" ]
```

Paste it exactly, starting with the `#cloud-config` line.

## 2. Create the server

1. Sign in at digitalocean.com, then **Create → Droplets**.
2. **Region:** one near you. Not every region offers every size; if you cannot
   find the **Basic** plan in step 4, go back and pick another region.
3. **Image:** Ubuntu **24.04 (LTS) x64**.
4. **Size:** **Basic → Regular → 2 GB / 1 CPU** (about $12/month) or bigger.
5. **Authentication:** choose **Password** and save it in your password manager.
6. Scroll to **Advanced Options** (open it), tick **Add Initialization scripts
   (free)**, and **paste** the setup text into the box that appears.
7. Click **Create Droplet**.

DigitalOcean's screens change from time to time. If a label differs, look for
"user data", "startup script" or "cloud-init".

## 3. Wait, then check it is ready

Installing takes **15 to 30 minutes**. Until then there is nothing to see: the
green "Active" dot only means the empty server is on.

1. Open your Droplet and click **Console** (top right). Click inside it and press
   **Enter**. If it asks you to log in, use `root` and your Droplet password.
2. **A box titled "Easy Bitcoin Payments" showing a web address and a SETUP CODE**
   means the install finished.
3. Just a plain `login:` prompt means it is still installing. Wait 5 minutes and
   look again.
4. A box saying "No setup code yet" or "still starting": wait 2 minutes, log in as
   `root`, type `ebp-info` and press Enter.
5. It is truly ready when the web address opens in your browser with a padlock
   and asks for the code. If it does not open, wait 5 minutes and try again.

If nothing has appeared after 30 minutes, log in and run `cloud-init status` and
`tail -n 40 /var/log/ebp-vps-setup.log`. If you share those lines, delete the one
containing the SETUP CODE. If the address never opens, check that a DigitalOcean
**Firewall** (if you made one) allows ports 80 and 443.

## 4. Finish setup in your browser

Open the web address from the box (it ends in `/setup`) and follow the page:

1. **Setup code.** Type the code from the box.
2. **Create wallet.** One button.
3. **Recovery words.** 12 words, shown for 60 seconds at a time. Write them on
   paper, twice, and keep the copies in different places. Never photograph them
   or type them anywhere else. You will be asked for three of them to prove you
   wrote them down. Anyone with these words can take the wallet's money.
4. **Store name and admin password.** The password must be at least 16
   characters (four random words works well). It signs you in to the admin page
   and locks your backups.
5. **Where your money goes.** Choose **Ark wallet** or **On-chain Bitcoin
   wallet**, open Receive in that wallet, copy an address and paste it. For play
   money it starts with `tark1` (Ark) or `tb1` (on-chain). This is required. You
   can change it later in Settings (a change waits a day for safety).
6. **API key.** Shown once. Save it in your password manager.
7. **Finish.** The server restarts once, so the page may not answer for about a
   minute. Then open `/admin` on the same address and sign in with your password.

## 5. Make a test payment

1. Admin page → **Payments** → **Try a payment** → **Create a test payment
   request** → **Open the checkout page**. It shows a QR code.
2. In your signet wallet choose Send, scan the QR and confirm. The page flips to
   **Paid** within seconds.

## 6. Connect your shop

Follow [WOOCOMMERCE.md](WOOCOMMERCE.md) (about 10 minutes). The admin page's
**Connect** tab gives you what the plugin asks for, one numbered step at a time.
Finish by sending a test notification and placing a small test order.

## 7. Keep a backup

Admin page → **Settings → Download a backup**. Copy the file somewhere that is not
the server. It is locked with your admin password, so download a fresh one after
changing the password. Details and how to restore:
[RECIPE_BACKUP.md](RECIPE_BACKUP.md).

## You are done when

- [ ] `/admin` opens after you sign in, and **Home** says everything is OK.
- [ ] A test payment shows **Paid** and appears under **Payments**.
- [ ] On the **Connect** tab, **Send a test notification** says "Your shop received the test."
- [ ] A small test order in your shop moved to Processing by itself.
- [ ] You downloaded a backup and stored it away from the server.
- [ ] Your recovery words are on paper in two places.

## Later

- **A nicer address.** The free `sslip.io` address is built from the server's IP,
  so it changes if you rebuild the server. For a fixed address, reserve an IP in
  DigitalOcean (**Networking → Reserved IPs**) or use your own domain: add an **A
  record** pointing at the server, then in the Console run
  `cd /opt/ebp && ./scripts/start.sh --domain pay.myshop.com`. Do this before
  connecting your shop; afterwards, update the server address in the plugin.
- **Updating.** Nothing updates itself. The admin Home page tells you when a new
  version exists: [UPGRADING.md](UPGRADING.md).
- **Something looks wrong:** [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md)
  (money going to the wrong place, a leaked password or key) and
  [KNOWN_ISSUES.md](KNOWN_ISSUES.md).
- **Start over:** before setup is finished, destroy the Droplet and make a new one.

## Other options

- **Another hosting company, Render, or your own computer:** [OTHER_HOSTS.md](OTHER_HOSTS.md).
- **Shopify:** not supported. WooCommerce comes first; Shopify is planned for a much later release. [Notes](dev/SHOPIFY.md).
- **Developers:** [dev/TECHNICAL.md](dev/TECHNICAL.md).

## Glossary

- **Signet:** a practice Bitcoin network with worthless coins.
- **Till:** the small wallet on your server that receives payments. EBP moves its
  contents to your own wallet automatically.
- **Recovery words:** 12 words that can rebuild the till wallet.
- **Ark:** the payment technology EBP uses. Leave its server at the default.
- **API key:** the password your shop uses to ask EBP for payments.
- **Webhook / signing secret:** the message EBP sends your shop when an order is
  paid, and the secret that proves it came from EBP.
