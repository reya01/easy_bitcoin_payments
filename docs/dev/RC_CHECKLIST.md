# Release candidate checklist

What to do for each release candidate (tag `v0.3.0-rcN` and so on). For what is proven on real money and
what is not, read [MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md) and
[KNOWN_ISSUES.md](../KNOWN_ISSUES.md). The older signet-era to-do lists and the 0.2.2-rc1 notes draft are in
[archive/RC_CHECKLIST_HISTORY.md](archive/RC_CHECKLIST_HISTORY.md).

Goal of every tag so far: a **release candidate** that testers can install (play
money by default, real money experimental) and that has a documented, short list
of things it does not do. **Not** a "ready for real money" release. The first one
was `v0.1.0-rc1`; the latest is `v0.3.2-rc1` (22 November 2026). Sections 3 and 4 below apply to each.

Nothing here is published until the owner publishes a release (the tag is made by it). Tags on a public
repository cannot be taken back cleanly, so check first.

## 1. What CI proves on every change to `main`

| Check | What it covers |
| --- | --- |
| build, tests, clippy, fmt | The Rust code, including unit tests |
| Browser end-to-end | Setup wizard, recovery-words quiz, admin login, Ark payment, no on-chain way to pay, outage and a hung wallet, restart, underpay, test payment, encrypted backup, payout change, every request matches barkd's pinned API spec |
| WooCommerce end-to-end | A real WordPress + WooCommerce: checkout redirect, forged notice refused, partial and full payment, repeat notice ignored, return link, shop-side tripwire, changed order total not auto-paid |
| Upgrade and restore | Previous build upgraded in place keeps data; encrypted backup, verify (wrong password refused) and restore |
| DigitalOcean rehearsal | The paste-in setup run on a clean Ubuntu 24.04 machine, minus the real internet certificate |
| Scripts and blueprints | shellcheck, cloud-init validity, Render blueprint safety rules |
| barkd API drift | Our client matches barkd 0.9.0's published API |
| Dependency audit | Known-vulnerability scan |
| Docker image | Builds for amd64 (x86-64 only for now; arm64 returns at v1.0) |
| Release dry run (manual) | Builds the binary, packages the plugin, checksums, smoke-tests the binary; publishes and signs nothing |

## 2. What a person still has to do

Record the result of each. A box ticked without a real run is worse than an open box. The signet drill, the
DigitalOcean and WooCommerce tests, phone alerts and the restore drill have been run (see
[MAINNET_RELEASE_STATUS.md](MAINNET_RELEASE_STATUS.md)). For every release candidate, run the quick drills in
that release's cut checklist on the test server. Still open:

- [ ] **A newcomer reads `START_HERE.md`** and gets to a paid test order without
      help. Write down every place they hesitate.
- [ ] Render: only if you want it listed as available. Otherwise leave it "planned".

## 3. Decisions for the owner

- [ ] **Public name.** The GitHub account name appears in the clone address in
      the docs and `cloud-init.yaml`, in the update-check address in
      `src/updates.rs`, in `scripts/install.sh`, and in the image name. Change it
      **before** a final release (after a rename, old addresses only redirect until
      someone else registers the old name). Commits made so far also carry the
      old name inside the saved author address; hiding that needs a history
      rewrite or a fresh repository. The owner chose to leave the existing history as it is; decide the name before 1.0.
- [ ] **Repository settings:** private vulnerability reporting (Settings, Code security; `SECURITY.md` points to
      it), secret scanning and push protection, a ruleset for `main`, protected `v*` tags, commit signing,
      and the README comparison-table wording.
- [ ] Delete stale branches, especially `mut-test` (never merge it).

## 4. Release steps (in order)

1. `main` is green and the manual **Release dry run** (Actions → release → Run
   workflow, on `main`) is green. Download the `dry-run-release-files` artifact
   and look at it: one `ebp-*.tar.gz` (x86-64), the plugin zip, `SHA256SUMS`.
   Container images count as dependencies too: the Dockerfile base images and
   the Caddy image in `docker-compose.yml` are pinned by digest (CI refuses an
   unpinned one), and Dependabot proposes new digests as PRs. Review those like
   any other dependency update, and do not merge one in the same release as a
   risky change without a dry run.
2. Move the `Unreleased` section of `CHANGELOG.md` under the new version's
   heading (for example `0.3.2-rc1`), and check the version matches the tag in `Cargo.toml`, `Cargo.lock`, and the
   plugin (`ebp-bitcoin.php` header and `EBP_BITCOIN_VERSION`, `readme.txt` Stable tag),
   so Home and the Plugins page show which build is running. The release
   workflow runs `scripts/check_release_version.sh vX.Y.Z` first and stops
   before building if any of these disagree with the tag (you can run it
   yourself first).
3. Tag the new version (for example `v0.3.2-rc1`) on `main` and push it. The
   `-rcN` suffix makes the workflow
   publish a **pre-release**, which GitHub's "latest" and the dashboard's update
   notice both ignore. Install an rc by explicit version, not `latest`.
4. Watch the release run. Then check: assets present, `SHA256SUMS` matches,
   the signature bundle exists, the notes show the image digest and the verify
   command, and the verify command works on your machine.
5. Install that release on a fresh server exactly as `START_HERE.md` says, using
   the digest-pinned image.
6. Only then tell testers. The final release (`v0.3.2`, the number the latest candidate is
   a candidate for) comes after the checklist in
   section 2 is fully ticked; the dashboard will tell rc users when it exists.

Release notes for each candidate are written in plain words from the `CHANGELOG.md` section, kept next to the
cut checklist (the project docs in Claude hold the latest one). Say plainly what is new, what is tried with real
money and what is not, and link `docs/KNOWN_ISSUES.md`.
