# Easy Bitcoin Payments

**Easy Bitcoin Payments lets an online store accept Bitcoin without a payment company in the middle.** You run a small server of your own, and a plugin connects it to your WooCommerce shop. Customers pay with Bitcoin on a simple checkout page (Lightning, Ark or regular on-chain), and the order is marked paid automatically. Your earnings are moved to a wallet that you control, and there are no software fees. It is free and open source, and for now it is for **testing**: play money by default, real bitcoin only in tiny amounts.

> **⚠ EXPERIMENTAL: NOT FOR REAL FUNDS.** This is pre-release software. Its wallet
> integration has not been audited and no independent security audit has been
> completed. Use signet (play money, the default). Real bitcoin (mainnet) can be
> switched on (`EBP_MODE=live`, see [START_HERE.md](docs/START_HERE.md#real-money-experimental))
> but has not yet been tried with real money: if you do, use only a few thousand
> sats you can afford to lose.

## Goals

Accept Bitcoin **while holding your own keys**, in a way that is as easy as
possible, even for someone who has never used a terminal. The aim is that a
non-technical shop owner can set it up and manage it entirely from a phone, and
that nobody else, not us and not a payment company, ever holds their money. We
trade some purist trust-minimization for that accessibility, on purpose, and we say
plainly where.

**Why Ark?** Ark makes a fundamentally lower-risk design possible for a small
business. Moving money off the server with regular Bitcoin transactions is only
worthwhile in larger amounts, but Ark lets the server sweep even tiny payments out
to your own wallet within minutes. Less bitcoin sitting on a server that is
connected to the internet means less to lose if it is ever compromised, while you
still hold your own keys. (In exchange, fast payments rely on the Ark server staying
online; if it ever disappears you can exit to regular Bitcoin on your own.)

**Our design stance.** AI is making software easier to attack, and in this
environment a hack is a matter of *when*, not *if*. We cannot fix that by "coding
harder": every program has bugs we have not found yet. So EBP is built around
choices that make a break-in matter less: keep little money on the server (Ark),
keep the keys with you, make payouts go to one fixed address that takes a day to
change, ask for a second lock (a code from your phone) before anything risky, and
keep the amount of code and number of moving parts small. (Example: after the PHP2Shell bugs showed that any PHP shop can be taken over, we made sure a hacked shop cannot move your money, and added a one-click pause and optional limits on the shop's key.) We still write careful
code and test it, but we plan as if something will go wrong and ask, "what is the
worst that could happen then?"

## What we do and do not protect against

Being honest about this matters more than sounding safe.

**Built to hold up if these happen:**
- *Your shop website is hacked.* The shop's key can create payment requests but cannot change where your money goes. You can pause new payments with one switch.
- *Someone learns your admin password.* With two-step sign-in on, they still need a code from your phone to sign in, change your payout address or change how much the server keeps, and a changed address waits a day before it is used.
- *The payment server is attacked.* It holds only a small amount at a time (the rest is moved to your own wallet), and nobody but you holds your keys.

**Not protected, so please plan for it:**
- Someone with full control of the payment server can take what it holds at that moment, and can read its keys.
- Your recovery words, phone or authenticator in the wrong hands. Lose the recovery words and nobody, including us, can recover them.
- A hacked shop can still show your customers a fake payment page. EBP cannot see that happen.
- Bugs in the software we build on (including the Ark server and wallet) and in ours. No independent security audit has been done yet, which is why this is play-money only.

The longer lists are in [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md) and [docs/dev/SHOP_COMPROMISE.md](docs/dev/SHOP_COMPROMISE.md).

## Get started (beginner path)

A small DigitalOcean server and a WooCommerce shop. No terminal needed.

1. **Create the server** by pasting our ready-made setup text into DigitalOcean.
2. **Wait 15 to 30 minutes**, then read the web address and setup code from the server's Console.
3. **Open that address** and follow the setup page: admin password, 12 recovery words, your payout wallet.
4. **Connect your shop** with the WooCommerce plugin.

Every click is spelled out in **[docs/START_HERE.md](docs/START_HERE.md)**.

## Other ways to run it

Another hosting company, Render, or your own computer: [docs/OTHER_HOSTS.md](docs/OTHER_HOSTS.md).
WooCommerce is the supported, first-class shop. Shopify is planned for a much later release ([notes](docs/dev/SHOPIFY.md)), and so is optional payment-address verification with Branta ([notes](docs/dev/BRANTA.md)).

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
is swept off the server within minutes once at least 1,000 sats are above the
1,000 it keeps, so with the default settings the server holds about 2,000 sats at
most at any moment. Then, if your wallet
supports it, have the phone wallet move funds to regular on-chain Bitcoin once they
pass a threshold you choose. You hold the keys the whole way, and as your balance
grows, more of it sits in colder storage. The full, blunt comparison:
[docs/dev/TECHNICAL.md](docs/dev/TECHNICAL.md#ebp-vs-btcpay-server-an-honest-comparison).

## More

- [docs/WOOCOMMERCE.md](docs/WOOCOMMERCE.md): connect a WooCommerce shop
- [docs/KEEPING_WORDPRESS_SAFE.md](docs/KEEPING_WORDPRESS_SAFE.md): keeping the shop itself safe
- [docs/RECIPE_BACKUP.md](docs/RECIPE_BACKUP.md): backups and restore
- [docs/RESTORE_DRILL.md](docs/RESTORE_DRILL.md): practice a restore on a new server, step by step
- [docs/UPGRADING.md](docs/UPGRADING.md): updating
- [docs/IF_SOMETHING_LOOKS_WRONG.md](docs/IF_SOMETHING_LOOKS_WRONG.md) and [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md)
- [SECURITY.md](SECURITY.md): reporting a vulnerability
- **Developers:** [docs/dev/TECHNICAL.md](docs/dev/TECHNICAL.md) (architecture, API, configuration), [CONTRIBUTING.md](CONTRIBUTING.md)

MIT licensed ([LICENSE](LICENSE)).
