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

## Ground rules

- **Doc precedence.** For current status and operations, `README.md`,
  `docs/dev/HANDOFF.md`, and the close-out block in `docs/dev/PHASE1_PLAN.md` are
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
  `src/db.rs`. Never edit a migration that has shipped.
- PRs describe what changed and why, and call out anything touching custody,
  secrets, or webhooks.

## Reporting security issues

Please do not open public issues for security problems. Use GitHub's private
security advisories on this repository (Security → Advisories) so fixes can
be coordinated before disclosure.