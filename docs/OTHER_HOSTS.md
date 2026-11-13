# Other ways to run it

The recommended path is [DigitalOcean](START_HERE.md). EBP is plain Docker, so
it is never tied to one company: anything that runs Docker works, and moving
later means restoring your encrypted backup on the new host. **Play money
(signet) by default, on any host;** real money is experimental (see
[START_HERE.md](START_HERE.md#real-money-experimental)).

## What you are trusting, on any host

1. **The host company.** It can see the server's disk (the small "till" wallet
   and your order history). Use a strong password and two-factor sign-in on that
   account.
2. **The code you run.** EBP never updates itself; nothing changes unless you
   press the button or run the command.
3. **Yourself.** Your admin password, your 12 recovery words and your backups.

The server only holds a small till; the rest of your money sits in a wallet whose
keys only you hold. See [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md)
for what to do on a bad day, and do a weekly check that what your shop marked as
paid matches what your own wallet received: it is the one check that does not
depend on the server telling the truth.

## Any other cloud server (Hetzner, AWS, and so on)

Create an Ubuntu 24.04 server with at least 2 GB of memory, open ports 80 and 443,
and paste the same setup text as in [START_HERE.md](START_HERE.md) if the host has
an "initialization script" or "user data" box. Otherwise log in and run:

```
git clone https://github.com/reya01/easy_bitcoin_payments.git
cd easy_bitcoin_payments
./scripts/start.sh --https
```

(Docker and git must be installed; put `sudo` in front if you see "permission
denied".) The first start compiles the software and is silent for 15 to 30
minutes. At the end the script prints the padlock address and the SETUP CODE.
Open the address, enter the code, and continue with step 4 of START_HERE. Use
`--domain pay.myshop.com` instead of `--https` for your own domain (point the name
at the server first). The choice is remembered.

If it says the padlock address is not answering, ports 80/443 are closed or (own
domain) the name does not point at the server yet. Fix that and run the same
command again; nothing is lost.

## Your own computer

Install Docker and git (Mac, Windows with WSL, or Linux), then:

```
git clone https://github.com/reya01/easy_bitcoin_payments.git
cd easy_bitcoin_payments
./scripts/start.sh
```

Open `http://localhost:8080` and enter the SETUP CODE the script prints. This is
for trying it out only: customers and your shop cannot reach `localhost`, and the
payment server refuses to send notices to a shop on your own network, so the
WooCommerce connection will not work. To reach a remote server's setup page
without a padlock address, make an SSH tunnel from your computer:
`ssh -L 8080:localhost:8080 user@your-server`, then open `http://localhost:8080`.

## Render (experimental)

Render looks after the server and the padlock for you. It is **untested live**.

> **Render needs a PAID plan.** EBP stores your wallet and order history on a disk
> that must survive restarts, and Render only offers such a disk on paid services.
> On a free plan **everything, including your wallet files, is erased on every
> restart.** The blueprints refuse a free plan; do not switch to one.

Updates are manual (**Manual Deploy**; never turn auto-deploy on). Deploy from
**your own copy (fork)** of the repository so nothing changes unless you change
it. Create a Render account with two-factor sign-in, fork the repository, choose
**New → Blueprint**, pick your fork and accept the paid plan with the disk. The
SETUP CODE is in the service's **Logs** tab (search for `SETUP CODE`), and the
address is at the top of the service page. Then open `/setup` there and continue
with step 4 of [START_HERE.md](START_HERE.md). After the first release you can use
the pinned image blueprint `deploy/render-image.yaml.template` instead of building
from source. Render, like any host, can see the server's disk.

## Make a stolen or tampered server hurt less

- **Pay out to an Ark address.** Payments then move to your wallet within
  minutes, so the server never holds much.
- **Update on purpose.** Read the release notes first.
- **Protect the host account** with two-factor sign-in and a unique password.

A tampered server could still change the payout address or show customers its own
address, which just looks like "money stopped arriving". Only your own wallet
catches that, which is why the weekly check matters most. The dashboard's weekly
"does your own wallet agree?" card and the WooCommerce plugin's warnings are
tripwires, not proof. If you suspect a problem, disable the Bitcoin payment method
in WooCommerce first, then investigate.
