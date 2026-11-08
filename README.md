# Easy Bitcoin Payments

**Easy Bitcoin Payments lets an online store accept Bitcoin without a payment company in the middle.** You run a small server of your own, and a plugin connects it to your WooCommerce shop. Customers pay with Bitcoin on a simple checkout page (Lightning, Ark or regular on-chain), and the order is marked paid automatically. Your earnings are moved to a wallet that you control, and there are no software fees. It is free and open source, and for now it is for **play-money testing only**.

> **⚠ EXPERIMENTAL: NOT FOR REAL FUNDS.** This is pre-release software. Its wallet
> integration has not been audited and no independent security audit has been
> completed. **Do not connect it to mainnet or real bitcoin.** Use signet
> (play money) until this notice is removed.

## Get started (beginner path)

A small DigitalOcean server and a WooCommerce shop. No terminal needed.

1. **Create the server** by pasting our ready-made setup text into DigitalOcean.
2. **Wait 15 to 30 minutes**, then read the web address and setup code from the server's Console.
3. **Open that address** and follow the setup page: admin password, 12 recovery words, your payout wallet.
4. **Connect your shop** with the WooCommerce plugin.

Every click is spelled out in **[docs/START_HERE.md](docs/START_HERE.md)**.

## Other ways to run it

Another hosting company, Render, or your own computer: [docs/OTHER_HOSTS.md](docs/OTHER_HOSTS.md).
Shopify is not supported yet ([notes](docs/dev/SHOPIFY.md)).

## More

- [docs/WOOCOMMERCE.md](docs/WOOCOMMERCE.md): connect a WooCommerce shop
- [docs/RECIPE_BACKUP.md](docs/RECIPE_BACKUP.md): backups and restore
- [docs/UPGRADING.md](docs/UPGRADING.md): updating
- [docs/IF_SOMETHING_LOOKS_WRONG.md](docs/IF_SOMETHING_LOOKS_WRONG.md) and [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md)
- [SECURITY.md](SECURITY.md): reporting a vulnerability
- **Developers:** [docs/dev/TECHNICAL.md](docs/dev/TECHNICAL.md) (architecture, API, configuration), [CONTRIBUTING.md](CONTRIBUTING.md)

MIT licensed ([LICENSE](LICENSE)).
