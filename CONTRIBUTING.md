# Contributing to Easy Bitcoin Payments

EBP is free, open-source software (MIT — see `LICENSE`). Contributions are
welcome: code, docs, testing, and especially feedback from non-technical
merchants.

## Design principle: say what and why, keep how in the background

Everything a shop owner or customer sees (screens, messages, docs) should be
direct and simple, and explain **what this does and why**, not **how** it works.
Technical detail is welcome, but it is never the default:

- Put it behind a hover hint, a "Help" or "Details" section, or a later page, so
  people who want it can find it and everyone else is not slowed down.
- Be exact where it helps: name the specific standard (for example BIP 321,
  BOLT 11, BIP 352) in that background detail, so an expert can verify it.
- Use plain words in the default text ("Bitcoin wallet app", not "URI"; "regular
  Bitcoin", not "on-chain"), with opinionated defaults, without ever weakening
  security or sovereignty (the merchant still holds the keys).

## Design principle: always say which place a step happens in

Setting up involves flipping between two different websites with similar
names: **your payment server** (the Easy Bitcoin Payments admin page) and **your
WooCommerce site** (the shop's WordPress admin). Every step in the docs, the
plugin's settings text and the dashboard that names a screen, tab, button or
box must say which of the two it is on, every time. Never write only
"the Connect tab" or "the settings page" and assume the reader knows which one.
Use those two names consistently, and say that they are different websites.

## Design principle: a beginner on a cheap server must never be stranded

Assume the person running EBP is a beginner on a small, cheap VPS with little or
no ability to fix a problem themselves. So:

- Nothing outside the server (a price website, a notification service, an update
  check) may be able to slow a page down or stop a payment. Look such things up in
  the background, store the last good answer, and only read the stored answer when
  a customer or the owner is waiting.
- Prefer one simple path over two special cases. Extra paths add ways to fail; a
  small inaccuracy in a rare edge case (for example an order priced a percent
  off the very latest price) is a better trade than another failure mode.
- When something does go wrong, keep working as far as possible, say so in plain
  words on Home, and tell the owner what to do (or that nothing is needed).
- No fixed or fake data in a "test mode" code path: test mode should exercise the
  same code that live mode will run.
- Give one curated easy path. Where a beginner must pick something, name the
  recommended choice and the exact next step: "install Noah on Android or Arké on
  iPhone", not "a wallet (Ark or on-chain)". Mention the alternatives briefly,
  after the easy path, for people who already have their own setup.

## People are running this: updates must not break them

EBP now has its first outside users. They update from one release candidate to
the next, often from a phone, and many cannot fix a broken server by hand. So:

- **Mark breaking changes clearly.** A change is breaking if, after updating, a
  server, shop plugin, saved setting, backup, API caller or habit stops working
  the way it did, or needs a step from the owner (a new setting, a changed
  `.env`, a plugin update at the same time, a migration that cannot be undone).
  Put **BREAKING:** at the start of the PR title and of its `CHANGELOG.md` line,
  say in plain words what the owner must do, and put that step in the release
  notes. Prefer a change that is not breaking: keep the old way working
  alongside the new one for at least one release when you can.
- **Test the update, not only the new code.** CI's `upgrade` job starts the
  last published release, gives it real data, stops it and starts this build on
  the same data folder (and restores a backup). A change to migrations, stored
  settings, the backup, the API or the plugin must pass it, and should add the
  data it touches to `tests/upgrade/run_upgrade.py` so the test proves that data
  survives. Keep the job's `last release` baseline on the newest release.
- **Migrations only add.** New tables and new columns with a default; never
  rename or drop a column, change what a stored value means, or edit a migration
  that has shipped. Old data must still read the same after the update.
- **API and plugin answers only add.** New fields are fine; removing or renaming
  a field, or changing its meaning, is breaking (an older shop plugin may still be
  calling). The same goes for CSV columns: add new ones at the end.
- **Say how it was tested.** The PR description lists the tests that cover the
  change, including the update path, and what still needs a person on a real
  server (add it to the next release's test sheet).

## Money code is written in Rust

Anything that touches money is written in Rust, in the payment server (`src/`).
Rust's type checks and memory safety, plus one place to review, make it the
safest home for the code that can lose someone's bitcoin.

**Money code** means anything that decides or changes:
- amounts and conversions (sats, prices, fees, rounding);
- whether something is paid, part paid, late or expired;
- matching a payment to an invoice or order;
- payout addresses and their safety delay;
- sending, sweeping or withdrawing;
- the wallet and the barkd client;
- balances;
- what gets written to the payment tables;
- the signatures and secrets that guard these.

**Other languages are fine for everything else:** tests, scripts, formatting
and docs, page design, and the WooCommerce plugin's job of talking to the shop.

**JavaScript is limited to design and ease of use.** It runs in the browser,
where extensions, injected scripts and a tampered page can change it. So:
- The pages (`assets/*.js`) may show, format and ask "are you sure?".
- They never decide anything about money. The server checks every amount,
  status and address again, and its answer is the one that counts.
- A number the page works out (a dollar estimate, a part-paid remainder, the
  "is this right?" check) is only a display or a convenience. Nothing may
  depend on it being right.
- If a page needs a money fact, add it to the server's answer rather than
  working it out in the browser.

**The shop plugin (PHP)** passes the order to the server and acts on the
server's signed answer. It keeps only its safety checks: it refuses a forged
notice, and it does not mark paid an order whose total changed. Any new money
decision goes in the server and is sent to the plugin as a result.

**Testing money code.** Besides ordinary tests, `src/money_properties.rs` runs thousands of
random cases against the money rules and compares them with a plain model; extend it when you
change a rule. **Mutation testing** changes the money code one line at a time (`>` to `>=`, a
function to `return 0`, ...) and lists every change the tests did not catch:
- On every PR that touches money code, the **mutants-pr** check mutates only the changed lines
  (minutes). For now it only reports; a "missed" mutant there means: add a test, or add it to
  `.cargo/mutants.toml` with a one-line reason why it does not matter.
- Before each release, the manual **mutants** workflow runs over all the money code (hours; it is
  step 1 of the release steps in `docs/dev/RC_CHECKLIST.md`).
- The goal is not a perfect score: it is that every mistake that could cost money would be caught.

**Where that is not possible** (for example, code that must run inside
WooCommerce), say so in the PR. Keep that code as small as possible, and test
it against the Rust server in CI.

## Ground rules

- **Browsers.** The checkout page, setup page and admin page are tested only in Google
  Chrome and Brave, with and without the Mullvad and Obscura VPNs. Other browsers are not
  supported and issues about them will be closed; pull requests that break Chrome or Brave
  are the ones that matter.
- **Doc precedence.** For current status and operations, `README.md`,
  `docs/dev/HANDOFF.md`, and the close-out block in `docs/dev/archive/PHASE1_PLAN.md` are
  authoritative. `spec.md` is the product intent; when the build supersedes a
  spec claim, update the spec in the same PR rather than treating the stale
  claim as a requirement.
- **Phase 0 is done; Phase 1 is code-complete.** The wallet layer is a real
  barkd client, live-verified on signet (docs/dev/HANDOFF.md is the status/ops
  entry point and records the drills). The EXPERIMENTAL / signet-only banner
  still applies: never wire real funds without the maintainer's go-ahead.
- **Out of scope by design:** refund tooling, watch-only/external-signing
  custody, Lightning send, Shopify, standalone storefront mode (see the
  spec's "Explicitly not v1.0" list). Don't build these in PRs.
- **No secrets in logs, ever.** API keys, webhook secrets, and seeds must
  never appear in log lines, error messages, or test output. If a change
  touches secret handling, call it out in the PR description.
- **UX gates are real.** No feature ships until a non-technical tester
  completes its flow unassisted; PRs touching user-facing flows should note
  how that will be tested.

## Development

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --all-features
```

CI runs all three on every push; a PR that fails them is not reviewed.

- Keep unit tests next to the logic they cover. Modules with logic (invoice
  state machine, auth, webhook signing, rates, migrations, watcher) carry
  their own tests.
- Migrations are append-only: add `migrations/000N_*.sql` and register it in
  `src/db.rs`. Never edit a migration that has shipped (see "People are running
  this" above).
- PRs describe what changed and why, and call out anything touching custody,
  secrets, or webhooks.

## Reporting security issues

Please do not open public issues for security problems. Use GitHub's private
security advisories on this repository (Security → Advisories) so fixes can
be coordinated before disclosure.