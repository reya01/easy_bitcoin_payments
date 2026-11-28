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

1. **Invoices and WooCommerce at a high level of stability.**
   - Every invoice path tried on real money in small amounts: Lightning, Ark, paid late, part paid, cancelled then paid, expired then fresh.
   - The WooCommerce plugin tested on current WordPress and WooCommerce, in both the block checkout and the classic checkout (classic is not separately tested today), with a real WordPress in the automatic checks.
   - Orders always end up right: paid, expired, cancelled, paid late, notice lost and resent. A load test with hundreds of simulated payments.
   - Invoices come out of "in testing". They stay off until the owner turns them on.
2. **Sovereign Hybrid Compute: a tested guide for advanced users.**
   - A written, step-by-step install on a Sovereign Hybrid Compute server, tried from scratch, and an update on it tried from the previous release.
   - It is the second supported host, as an advanced route. DigitalOcean stays the easy path unless the provider offers a paste-in script or an API.
   - Before writing it we find out what they actually offer (server locations, a standard Linux image, startup scripts, an API, and whether they accept bitcoin). Their site does not say.

*Gate:* 30 days of real use on the owner's own shop with no money-affecting bug. The Sovereign Hybrid Compute guide followed from scratch by someone other than its writer, ending in a paid test order.

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

### v0.7: A steady wallet foundation

*The goal: updates from Second's Bark never put your money at risk.*

1. **Follow Bark on purpose.** Move to the current Bark by one written recipe: backup first, the wallet identity unchanged, a restore from the 12 words, one small send that must arrive as an Ark payment. A weekly check warns us when Bark changes something we use.
2. **Second's open questions answered** (whether a missed Lightning claim can be retried, how long a client works after the server raises its minimum version) and the code or docs updated to match.

*Gate:* a Bark move done by the recipe with an identical wallet before and after, and a restore that works on the new Bark.

### v0.8: Harder to lose

*The goal: a bad day costs little.*

1. **Easier, safer backups.** An opt-in backup that happens on its own to a place the owner chooses (today the encrypted backup is a manual download). Off by default, no new outside connection unless the owner sets it up.
2. **ARM servers return** (smaller, cheaper hosts), installed and tried.

*Gate:* a server destroyed and rebuilt from the automatic backup and the 12 words; an install on ARM that reaches a paid order.

### v0.9: The 1.0 candidate

*The goal: nothing new, everything proven.*

1. **Feature freeze.** Only fixes. Every drill (pause and resume, "Move my money now", restore, a small emergency withdrawal) repeated on the exact release.
2. **Docs and newcomer pass.** Both hosts (DigitalOcean, Sovereign Hybrid Compute) tested from scratch and by update on that release; every doc step says where it happens (payment server, shop site or phone wallet).

*Gate:* every box on the 1.0 list below is ticked except the one waiting on Bark 1.0.

### v1.0: A promise

1.0 is a promise of stability and honesty, not a feature count. It is tagged only when:

- EBP runs on Bark 1.0 or a later 1.x, and the real-money drills were repeated on that pair, then a 30-day soak.
- Every rescue path has been tried on real money on the exact release.
- The independent review is done and a second operator has run it.
- Releases are signed, rebuild identically, and update from the previous release keeps all data.
- WooCommerce and invoices are tested on two hosts.
- The owner decides whether "experimental" comes off the README.

Backups, the shop connection and the settings names keep working across 1.x. A change that cannot is a 2.0.

## After 1.0

One at a time, only when WooCommerce and invoices are solid:

- **Shopify.** Later than first hoped: Shopify's recent changes make outside payments harder, so it needs its own careful plan ([docs/dev/SHOPIFY.md](dev/SHOPIFY.md)).
- Branta address checks, Silent Payments payouts, Lightning payouts, refunds, and automatic updates.
- More hosts (Render and others).

## What is not promised

Shopify before 1.0, refunds (bitcoin payments are final), more than two hosts before 1.0, browsers other than Chrome and Brave, and big balances: the server keeps only a small till by design.
