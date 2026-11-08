# Easy Bitcoin Payments

**Easy Bitcoin Payments lets an online store accept Bitcoin without a payment company in the middle.** You run a small server of your own, and a plugin connects it to your WooCommerce shop. Customers pay with Bitcoin on a simple checkout page (Lightning, Ark or regular on-chain), and the order is marked paid automatically. Your earnings are moved to a wallet that you control, and there are no software fees. It is free and open source, and for now it is for **play-money testing only**.

> **⚠ EXPERIMENTAL: NOT FOR REAL FUNDS.** This is pre-release software. Its wallet
> integration has not been audited and no independent security audit has been
> completed. **Do not connect it to mainnet or real bitcoin.** Use signet
> (play money) until this notice is removed.

## Goals

Accept Bitcoin **while holding your own keys**, in a way that is as easy as
possible, even for someone who has never used a terminal. The aim is that a
non-technical shop owner can set it up and manage it entirely from a phone, and
that nobody else, not us and not a payment company, ever holds their money. We
trade some purist trust-minimization for that accessibility, on purpose, and we say
plainly where.

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

## How it compares to BTCPay Server

BTCPay Server is the established self-hosted Bitcoin payment processor, and it is
the right tool for many merchants. EBP shares its mission (self-hosted, you hold
the keys, no middleman) but aims at a different merchant.

| | BTCPay Server | EBP |
| --- | --- | --- |
| Stack | C#/.NET, years of accumulated features | Rust, a single binary plus SQLite |
| Payment rails | Lightning channels and on-chain | Ark (Lightning and on-chain also supported) |
| Inbound liquidity | You must arrange it | None needed |
| Bitcoin node | Full node required | None |
| Upkeep | Chain sync, channel management, several services | One server, no chain sync |
| Money on the server | Lightning keys and channel funds | A small capped "till", swept to your wallet automatically |
| Setup | Sysadmin-grade | Paste one block of text, no terminal |
| Lightning trust | No third party on the payment path | Depends on the Ark server |
| Features | POS, crowdfunding, pay buttons, plugins | Invoice API, checkout page, WooCommerce plugin |
| Maturity | Thousands of merchants in production | Pre-release, unaudited |
| Managed from | Desktop first | Phone friendly |

**Where BTCPay wins:** real Lightning channels with no third party in the payment
path, years of production use, and a much broader ecosystem. If you want maximum
trust-minimization, or you already run a node, use BTCPay.

**Where EBP wins:** no node, no channels and no liquidity to manage; setup a
beginner can finish; and a smaller loss if the server is ever compromised.

We do **not** claim EBP is more secure than BTCPay Server. It is **lower risk, on
purpose**: the server only ever holds a small amount of money, so less is in harm's
way. A smaller prize is not a stronger vault; it is a different bet.

For example, set your payout address to an Ark wallet on your phone. Every payment
is swept off the server within minutes, so even at the default limit of 20,000
sats, that is the most the server holds at any moment. Then, if your wallet
supports it, have the phone wallet move funds to regular on-chain Bitcoin once they
pass a threshold you choose. You hold the keys the whole way, and as your balance
grows, more of it sits in colder storage. The full, blunt comparison:
[docs/dev/TECHNICAL.md](docs/dev/TECHNICAL.md#ebp-vs-btcpay-server-an-honest-comparison).

## More

- [docs/WOOCOMMERCE.md](docs/WOOCOMMERCE.md): connect a WooCommerce shop
- [docs/RECIPE_BACKUP.md](docs/RECIPE_BACKUP.md): backups and restore
- [docs/UPGRADING.md](docs/UPGRADING.md): updating
- [docs/IF_SOMETHING_LOOKS_WRONG.md](docs/IF_SOMETHING_LOOKS_WRONG.md) and [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md)
- [SECURITY.md](SECURITY.md): reporting a vulnerability
- **Developers:** [docs/dev/TECHNICAL.md](docs/dev/TECHNICAL.md) (architecture, API, configuration), [CONTRIBUTING.md](CONTRIBUTING.md)

MIT licensed ([LICENSE](LICENSE)).
