# Choosing where to run your payment server

Short version: **today, use a DigitalOcean server** ([guide](VPS_DIGITALOCEAN.md)).
**Render** is planned as a second beginner-friendly option and may become the
first one we recommend if testing shows it is substantially easier. EBP is
plain Docker, so it is never tied to one company: anything that runs Docker
works, and moving later means restoring your encrypted backup on the new host.

**Test version:** play-money (signet) only for now, on any host.

## What you are trusting, on every host

On any option below you are trusting:

1. **The host company.** It can see the server's disk, which holds the small
   "till" wallet and your order history, and anyone who takes over your account
   with the host can get to it too. Use a strong password and two-factor sign-in
   on that account.
2. **The code you choose to run.** EBP never updates itself. You press the
   button (or run the command) yourself, so a bad change upstream cannot reach
   your server unless you deploy it.
3. **Yourself.** Your admin password, your 12 recovery words, and keeping
   backups.

The design keeps the stakes small: the server only holds a small till, and
the rest of your money sits in a wallet whose keys only you hold.

| | DigitalOcean server | Render |
| --- | --- | --- |
| Status | Guide written; setup rehearsed in CI; first real run still to do | Blueprint and guide drafted ([RENDER.md](RENDER.md)); not tested live yet |
| Trust model | Host + code you choose to run | The same |
| Who patches the operating system | You (keep the server and Docker updated) | Render |
| Padlock (https) | Automatic, free `sslip.io` address or your domain | Built in, `onrender.com` address or your domain |
| Updating EBP | Run one command | Press **Manual Deploy** |
| Where you find the setup code | On the server's console screen | In the service's **Logs** tab |
| Needs a paid plan | Yes (about $12/month, 2 GB) | Yes: the free plan erases the wallet on restart |
| Can you move away later | Yes (restore your backup elsewhere) | Yes (the same) |

We have not ranked them beyond "do DigitalOcean first". We will test both the
same way, with a non-technical person on play money and nobody helping, and
write down which is easier. If Render is substantially easier it becomes the
first option we list. DigitalOcean stays supported either way.

## Make a stolen or tampered server hurt less

The payout settings stay as they are. These choices only make a bad day cheaper.
Neither host changes them.

- **Pay out to an Ark address with a low limit.** Every payment is then moved
  to a wallet you hold within minutes, so the server never holds much. The
  default on-chain payout is the simplest and most widely understood, and
  you can watch an on-chain address with any public address watcher. If you
  choose on-chain, keep the limit sensible too.
- **Update on purpose.** Deploy only releases you have read about, never
  "whatever is newest". Check the release notes first.
- **Protect the host account.** Two-factor sign-in, a unique password.
- **Know what to do on a bad day.** [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md) has the short list; practise it once on play money.
- **Check your own wallet.** About once a week, compare the payments your shop
  marked as paid with what your wallet actually received. This is the one
  check that does not depend on the payment server telling the truth.

## What a tampered server could still do

Be clear about the limits. If someone put malicious code on your server, a
small payout limit would not stop them from:

- changing the payout address so that new payments go to them (you would lose
  revenue until you noticed, not just the till), or
- showing your customers their address at checkout, so the money never touches
  your server at all.

Both look like "money stopped arriving", and only a check outside the server
catches them: your own wallet. So the weekly check above matters more than any
setting. The dashboard now shows a weekly "does your own wallet agree?" card
with what the server claims, and the WooCommerce plugin warns (in the
WordPress admin and by email) when Bitcoin orders stop getting paid or the
server stops answering. These are tripwires, not proof: a fully compromised
server could fake some of what it reports, which is why the check against your
own wallet matters most. If you ever suspect a problem, disable the Bitcoin payment
method in WooCommerce first, then investigate.
