# Put your payment server on a DigitalOcean server

This is our recommended way to run Easy Bitcoin Payments (tested by an automated
rehearsal, not yet on a real DigitalOcean server): a small always-on server you
rent for about $6–12 a month. You do **not** need to use a terminal. You paste
one block of text when you create the server, wait, and then the server's own
screen tells you where to go next.

**Test version:** use play-money (signet) only for now.

Not sure it's the right host? [CHOOSING_A_HOST.md](CHOOSING_A_HOST.md) compares the options. EBP is plain Docker, so you are never locked in.

Why DigitalOcean: the sign-up is quick, it takes cards and PayPal, it has a
built-in browser "Console" (so no SSH or terminal app), and it lets you paste
setup instructions while creating the server. Hetzner works the same way but
is not covered here yet.

## 1. Create the server ("Droplet")

1. Create an account at digitalocean.com and add a payment method.
2. Click **Create → Droplets**.
3. **Region:** the one nearest you.
4. **Image:** Ubuntu **24.04 (LTS) x64**.
5. **Size:** Basic → Regular → **2 GB / 1 CPU (about $12/month)** or bigger.
   (Until the first official release exists, the server builds the software
   itself, which needs the memory. After a release, 1 GB is enough.)
6. **Authentication:** pick **Password** and choose a strong one, or an SSH key
   if you know what that is. You will mostly not need it.
7. Open **Advanced Options** → tick **Add Initialization scripts (free)**
   and paste the entire contents of
   [`deploy/cloud-init.yaml`](../deploy/cloud-init.yaml).
8. Click **Create Droplet**.

## 2. Wait, then read the server's screen

The server installs everything by itself. Building the first time takes
**15–30 minutes** (a few minutes once an official release exists).

When it's ready:

1. Open your Droplet in DigitalOcean and click **Console** (top right).
2. The Console opens in a new window. It shows a box with **a web address**
   and a **SETUP CODE**. If it asks you to log in, sign in as `root` with the
   password you chose; the same box is also shown on the login screen.
3. If the box says the server is still starting, wait two minutes, then type
   `ebp-info` and press Enter.

## 3. Finish setup in your browser

Open the web address from the box (it looks like
`https://203-0-113-7.sslip.io/setup`, with a padlock), type the SETUP CODE and
follow the steps. [START_HERE.md](START_HERE.md) explains each one, including
writing down your 12 recovery words.

## Optional: keep the same address, or use your own

The free `sslip.io` address is made from your server's IP address. That is fine
for testing, but two things are worth knowing before you connect a real shop:
the address looks unfamiliar to customers, and if you ever rebuild the server it
gets a new IP and therefore a new address.

Do either of these **before** you connect your shop (it is easiest before the
setup page too). You type one line in the Console.

**Keep the same address (Reserved IP).** DigitalOcean gives a fixed IP address
that stays yours even if you replace the server (free while it is attached to a
server).

1. In DigitalOcean: **Networking → Reserved IPs**, reserve one and assign it to
   your Droplet.
2. In the Console, run the line below, replacing the numbers with your
   reserved IP and writing dashes instead of dots (203.0.113.7 becomes
   203-0-113-7):

   ```
   cd /opt/ebp && ./scripts/start.sh --domain 203-0-113-7.sslip.io
   ```

**Use your own domain** (best for a real shop, for example `pay.myshop.com`).

1. At the company that holds your domain (or in DigitalOcean **Networking →
   Domains**), add an **A record**: name `pay`, pointing at your server's IP
   (the reserved IP if you made one).
2. Wait a few minutes, then in the Console run:

   ```
   cd /opt/ebp && ./scripts/start.sh --domain pay.myshop.com
   ```

The command prints your new address. It is remembered, so a later `ebp-info`
or restart keeps it. If you had already connected your shop, update the server
address in the WooCommerce plugin and sign in to the dashboard again at the new
address.

## If something goes wrong

- **The address doesn't open:** wait a few minutes; the padlock certificate is
  requested on first start. If you turned on a DigitalOcean **Firewall**, it
  must allow ports 80 and 443.
- **The box never appears:** first make sure at least 30 minutes have passed
  since you created the Droplet. Then log in to the Console as `root` and type
  `ebp-info`. If it says "command not found", the install did not finish: type
  `cloud-init status` (it says "running" while still working) and
  `tail -n 40 /var/log/ebp-vps-setup.log`, and send us those lines (they
  contain no passwords or recovery words, but they do show your one-time SETUP
  CODE, so leave that line out).
- **Start over:** destroy the Droplet and make a new one (nothing important is
  on it before setup is finished).

## Updating, backups and costs

- Updates are never automatic. The dashboard tells you when one exists; the
  update steps are in [UPGRADING.md](UPGRADING.md).
- Download a backup from **Settings** regularly; keep your 12 recovery words on
  paper.
- Cost is the Droplet price only. The padlock address (`sslip.io`) and the
  certificate are free, and a Reserved IP costs nothing while it is attached.
  A domain name of your own costs whatever your registrar charges (see above).
