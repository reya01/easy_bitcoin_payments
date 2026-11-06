# Put your payment server on a DigitalOcean server

This is our recommended, tested way to run Easy Bitcoin Payments for real: a
small always-on server you rent for about $6–12 a month. You do **not** need
to use a terminal. You paste one block of text when you create the server, wait,
and then the server's own screen tells you where to go next.

**Test version:** use play-money (signet) only for now.

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
2. The login screen shows a box with **a web address** and a **SETUP CODE**.
   You do not need to log in.
3. If the box says the server is still starting, wait two minutes and press
   Enter on the console to refresh, or log in as `root` and type `ebp-info`.

## 3. Finish setup in your browser

Open the web address from the box (it looks like
`https://203-0-113-7.sslip.io/setup`, with a padlock), type the SETUP CODE and
follow the steps. [START_HERE.md](START_HERE.md) explains each one, including
writing down your 12 recovery words.

## If something goes wrong

- **The address doesn't open:** wait a few minutes; the padlock certificate is
  requested on first start. If you turned on a DigitalOcean **Firewall**, it
  must allow ports 80 and 443.
- **The box never appears:** log in to the Console as `root`, type `ebp-info`.
  The full log is in `/var/log/ebp-vps-setup.log`.
- **Start over:** destroy the Droplet and make a new one (nothing important is
  on it before setup is finished).

## Updating, backups and costs

- Updates are never automatic. The dashboard tells you when one exists; the
  update steps are in [UPGRADING.md](UPGRADING.md).
- Download a backup from **Settings** regularly; keep your 12 recovery words on
  paper.
- Cost is the Droplet price only. The padlock address (`sslip.io`) and the
  certificate are free. You can use your own domain later:
  `./scripts/start.sh --domain pay.myshop.com`.
