# Contributing to Easy Bitcoin Payments

EBP is free, open-source software (MIT — see `LICENSE`). Contributions are
welcome: code, docs, testing, and especially feedback from non-technical
merchants.

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