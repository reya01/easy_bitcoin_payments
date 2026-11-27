# Use your own address (in testing)

Your server starts on a free address made from its IP, like `https://203-0-113-7.sslip.io`. When you are
comfortable with EBP, you can move to your own domain, like `https://pay.myshop.com`.

**Get your shop running first.** Domains are a common place for things to go wrong, so this is in testing and
turned off until you turn it on: Settings, **Use your own address (in testing)**.

Nothing changes on your server until a check passes. The old address keeps working the whole time as a backup,
and you can go back to it with one tap.

## The easy path: Cloudflare

You need a domain whose DNS is at Cloudflare. A free Cloudflare account is enough, and EBP never asks for your
Cloudflare password or any key.

- **No domain yet?** Sign up at [cloudflare.com](https://www.cloudflare.com), then buy one with Cloudflare
  Registrar (at cost, with your contact details hidden). Its DNS is already at Cloudflare.
- **Your shop's domain is already at Cloudflare?** Use `pay.` in front of it, for example `pay.myshop.com`.
  EBP suggests that name for you.
- **Your domain is at another company (GoDaddy, Namecheap, your web host)?** Use the
  [other providers](#any-other-dns-provider) steps there. Don't move your shop's domain to Cloudflare just for
  this: a missed email or website record would break your shop.

### Steps

1. **Optional but recommended on DigitalOcean: keep the IP.** In DigitalOcean, open **Networking**, then
   **Reserved IPs**, and reserve your Droplet's IP. Then rebuilding the server never changes its address.
2. On your server's admin page, open **Settings**, then **Use your own address (in testing)**, and tap **Turn
   this on**.
3. Type the name, for example `pay.myshop.com`. The page shows the one record to add, with Copy buttons.
4. In Cloudflare: open [dash.cloudflare.com](https://dash.cloudflare.com), pick your domain, open **DNS**, then
   **Records**, then **Add record**.
   - **Type:** A
   - **Name:** the first part of your name (`pay`)
   - **IPv4 address:** the address shown on the admin page (your server's IP)
   - **Proxy status:** turn the orange cloud **off**, so it says **DNS only**. This matters: your server gets
     its own padlock certificate, and the proxy would get in the way.
   - Save.
5. Back on the admin page, tap **Check**. It tells you, in plain words, what it found:
   - **Ready.**
   - **Not there yet:** wait a few minutes and check again. A brand-new Cloudflare domain can take up to 24 hours.
   - **Goes through Cloudflare's proxy:** turn the orange cloud off.
   - **Points somewhere else:** fix the IPv4 address.
   - **An IPv6 (AAAA) record points elsewhere:** delete that record.
6. Tap **Serve pay.myshop.com next to my current address**. Your server gets the padlock certificate for it,
   usually in under a minute. Both addresses now work: try `https://pay.myshop.com/admin` in a new tab.
7. Tap **Make pay.myshop.com my main address**. New payment links and QR codes use it. Links you already sent
   keep working on the old address. **Passkeys you made on the old address only work there:** add a passkey on
   the new address afterwards, or sign in with your authenticator app code.
8. **Update your shop:** in WordPress, open **WooCommerce**, then **Settings**, then **Payments**, then
   **Bitcoin**, and change **Server address** to `https://pay.myshop.com`. Until you do, the shop keeps using the
   old address, which still works.

## What happens next

- **For 14 days** your server checks the new address several times a day: it must still point at this server
  and have a valid padlock certificate. If a check fails, you get an alert (when phone alerts are on), the
  backup is kept, and Settings offers **Go back to the backup address**.
- **After 30 days**, if every check passed and your shop uses the new address, the free sslip.io address is
  given up. We don't hold on to a free service we no longer need. You are warned 3 days before, with a
  **Keep the backup a little longer** button.
- **Go back at any time** before then with one tap: the old address becomes the main one again. Your own
  address keeps working, and you can try again later.

## Any other DNS provider

The same screen and check work with any DNS provider. Add this record where your domain's DNS is managed:

- **Type:** A
- **Host or name:** the first part of your name (`pay`), or `@` for the bare domain
- **Value or points to:** your server's IPv4 address
- **TTL:** the lowest offered, or automatic
- If the provider has a proxy or CDN switch for the record, turn it off.
- Delete or fix any **AAAA** (IPv6) record on that name that points somewhere else.
- If your domain has **CAA** records, one of them must allow `letsencrypt.org`.

## Without the admin page (Console)

The admin page changes the address through a small helper on the server (installed with the update helper:
`sudo ./scripts/install-update-helper.sh`). Without it, in the DigitalOcean Console:

```
cd /opt/ebp
./scripts/start.sh --domain pay.myshop.com
```

This also keeps the sslip.io address as a backup. Add the record and wait for **Check** to pass first, so a
typo can't leave you with an address that doesn't work.

## Advanced setups

Cloudflare's proxy (orange cloud) is possible but not the guided path. It sends every checkout and admin page
through Cloudflare. If you use it, set Cloudflare's SSL/TLS mode to **Full (strict)**, which accepts your
server's own Let's Encrypt certificate. Your own reverse proxy, a tunnel, or another host: see
[OTHER_HOSTS.md](OTHER_HOSTS.md).
