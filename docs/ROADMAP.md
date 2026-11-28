# Roadmap

Plain and honest. **Order, not dates.** Every stage ends at a **gate**: a test
someone can run, not a day on the calendar. A stage is finished when its gate
passes. The owner decides what ships. Detailed plans live in `docs/dev/`.

Rules that shape every stage:

- **Not losing money comes first.** A feature waits if its money path has not been tried.
- **One easy path for beginners** (DigitalOcean, Noah on Android, Arké on iPhone).
  Other routes are for advanced users, and are labelled that way.
- **Money code is Rust.** JavaScript is for design only.
- **No tracking.** Nothing new phones home.
- A change that breaks something for existing users is marked **BREAKING** in the
  release notes and is tested before it ships.

## Where we are

| | |
| --- | --- |
| **Live: WooCommerce** | Lightning and Ark checkout, automatic payout to your own wallet, backups, one-tap updates, emergency withdrawal, releases you can rebuild and check. Still experimental: start small. |
| **In testing: invoices** | Get paid without a web shop: make an invoice, send it as text or a picture, paid by Lightning or Ark within 24 hours. Off until you turn it on in Settings. Plan: [docs/dev/INVOICES.md](dev/INVOICES.md). |
| **Next: 0.3.4** | Use your own address (for example `pay.myshop.com`) instead of the free one, behind a Settings switch, plus extra safety checks on payout changes. |

## The way to 1.0

### v0.4: Steady shops and invoices

*The goal: a shop owner or an invoice sender can rely on it every day.*

1. **Invoices at a high level of stability.**
   - Every invoice path tried on real money in small amounts: Lightning, Ark, paid late, part paid, cancelled then paid, expired then fresh.
   - Invoices come out of "in testing". They stay off until the owner turns them on.
2. **WooCommerce at a high level of stability.**
   - The plugin tested on current WordPress and WooCommerce, in both the block checkout and the classic checkout (classic is not separately tested today), with a real WordPress in the automatic checks.
   - Orders always end up right: paid, expired, cancelled, paid late, notice lost and resent. A load test with hundreds of simulated payments.

*Gate:* 30 days of real use on the owner's own shop with no money-affecting bug, and every invoice and order path above tried.

### v0.5: Checkable by others

*The goal: you do not have to take our word for it.*

1. **An independent security review** of the code that handles money, keys and sign-in, plus a current threat model. Every finding is fixed or written in [KNOWN_ISSUES.md](KNOWN_ISSUES.md).
2. **Someone else rebuilds a release** and gets the same files, and releases are also signed with a key the maintainer holds. (Releases already rebuild byte for byte on two machines inside GitHub; this adds a person outside it.)
3. Smaller: invoice polish (duplicate an invoice, print or save as PDF) and a signed "paid" event for people who connect their own tools.

*Gate:* review done and findings closed; two independent rebuilds match the published image.

### v0.6: Someone else runs it

*The goal: it works for people who are not us.*

1. **A second operator** who is not the owner gets from START_HERE to a paid order without help, then completes an update.
2. **Seen across time.** Coin renewals watched over several coin lifetimes, and the weekly "does your wallet agree?" check shown to catch a changed payout address.

*Gate:* the second operator finishes both without help; renewals run a full cycle with no manual fix.

### v0.7: A steady foundation, safer backups

*The goal: Bark updates and bad days cost you nothing.*

1. **Follow Bark on purpose.** Move to the current Bark by one written recipe: backup first, the wallet identity unchanged, a restore from the 12 words, one small send that must arrive as an Ark payment. A weekly check warns us when Bark changes something we use. Second's open questions answered (whether a missed Lightning claim can be retried, how long a client works after the server raises its minimum version) and the code or docs updated to match.
2. **Easier, safer backups.** An opt-in backup that happens on its own to a place the owner chooses (today the encrypted backup is a manual download). Off by default, no new outside connection unless the owner sets it up.

*Gate:* a Bark move with an identical wallet before and after and a working restore; a server destroyed and rebuilt from the automatic backup and the 12 words.

### v0.8: More places to run it

*The goal: not tied to one host or one kind of server.*

1. **Sovereign Hybrid Compute: a tested guide for advanced users.** A written, step-by-step install on a Sovereign Hybrid Compute server, tried from scratch and by update from the previous release. DigitalOcean stays the easy path unless the provider offers a paste-in script or an API. First we find out what they offer (server locations, a standard Linux image, startup scripts, an API, whether they accept bitcoin); their site does not say.
2. **ARM servers return** (smaller, cheaper hosts), installed and tried.

*Gate:* the guide followed from scratch by someone other than its writer, ending in a paid test order, and an ARM install that does the same.

### v0.9: The 1.0 candidate

*The goal: nothing new, everything proven.*

1. **Feature freeze.** Only fixes. Every drill (pause and resume, "Move my money now", restore, a small emergency withdrawal) repeated on the exact release.
2. **Docs and newcomer pass.** Both hosts (DigitalOcean, Sovereign Hybrid Compute) tested from scratch and by update on that release; every doc step says where it happens (payment server, shop site or phone wallet).
3. **An advanced option: a whole shop on DigitalOcean, with an AI assistant's help** (first draft: [SHOP_ON_DIGITALOCEAN.md](SHOP_ON_DIGITALOCEAN.md)). The recommended default stays a hosted WooCommerce shop such as WooCommerce.com; this is for people who want to run the shop themselves.
   - One tested recipe a person can hand to an AI assistant (or follow by hand) to go from an empty DigitalOcean account to a WooCommerce store **and** an Easy Bitcoin Payments server, connected, with a paid test order. It builds on DigitalOcean's own WordPress 1-Click image. An untested startup-script idea is kept in the repository for later.
   - Two servers, one for the shop and one for the payment server, so a hacked shop cannot reach the payment server ([docs/dev/SHOP_COMPROMISE.md](dev/SHOP_COMPROMISE.md)).
   - Written for beginners and for phones, and says which place each step happens in. The assistant never sees your recovery words, admin password, API key or signing secret: you type those yourself.
   - Tried with more than one AI assistant, and by someone who did not write it.

*Gate:* the recipe works from an empty account to a paid test order for someone who did not write it, and every box on the 1.0 list below is ticked except the one waiting on Bark 1.0.

### v1.0: A promise

1.0 is a promise of stability and honesty, not a feature count. It is tagged only when:

- EBP runs on Bark 1.0 or a later 1.x, and the real-money drills were repeated on that pair, then a 30-day soak.
- Every rescue path has been tried on real money on the exact release.
- The independent review is done and a second operator has run it.
- Releases are signed, rebuild identically, and update from the previous release keeps all data.
- WooCommerce and invoices are tested on two hosts.
- The advanced DigitalOcean shop recipe (shop and payment server, with an AI assistant) has been followed to a paid order by someone who did not write it.
- The owner decides whether "experimental" comes off the README.

Backups, the shop connection and the settings names keep working across 1.x. A change that cannot is a 2.0.

## After 1.0

One at a time, only when WooCommerce and invoices are solid:

- **Shopify.** Later than first hoped: Shopify's recent changes make outside payments harder, so it needs its own careful plan ([docs/dev/SHOPIFY.md](dev/SHOPIFY.md)).
- Branta address checks, Silent Payments payouts, Lightning payouts, refunds, and automatic updates.
- More hosts (Render and others).

## What is not promised

Shopify before 1.0, refunds (bitcoin payments are final), more than two hosts before 1.0, browsers other than Chrome and Brave, and big balances: the server keeps only a small till by design.
