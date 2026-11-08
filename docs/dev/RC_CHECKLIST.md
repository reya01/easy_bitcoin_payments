# Release candidate checklist

Goal of the first tag: a **release candidate** (`v0.1.0-rc1`) that testers can
install with play money (signet) and that has a documented, short list of
things it does not do. **Not** a "ready for real money" release.

Nothing here is published until a person pushes a tag. Tags on a public
repository cannot be taken back cleanly, so the last section lists exactly
what to check first.

## 1. What CI proves on every change to `main`

| Check | What it covers |
| --- | --- |
| build, tests, clippy, fmt | The Rust code, including unit tests |
| Browser end-to-end | Setup wizard, recovery-words quiz, admin login, Ark payment, on-chain payment and replacement, outage and a hung wallet, restart, underpay, test payment, encrypted backup, payout change, every request matches barkd's pinned API spec |
| WooCommerce end-to-end | A real WordPress + WooCommerce: checkout redirect, forged notice refused, partial and full payment, repeat notice ignored, return link, shop-side tripwire, changed order total not auto-paid |
| Upgrade and restore | Previous build upgraded in place keeps data; encrypted backup, verify (wrong password refused) and restore |
| DigitalOcean rehearsal | The paste-in setup run on a clean Ubuntu 24.04 machine, minus the real internet certificate |
| Scripts and blueprints | shellcheck, cloud-init validity, Render blueprint safety rules |
| barkd API drift | Our client matches barkd 0.8.0's published API |
| Dependency audit | Known-vulnerability scan |
| Docker image | Builds for amd64 and arm64 |
| Release dry run (manual) | Builds both binaries, packages the plugin, checksums, smoke-tests the binary; publishes and signs nothing |

## 2. What CI cannot prove (a person must do these)

Record the result of each. A box ticked without a real run is worse than an
open box.

- [ ] **Signet drill** (`SIGNET_DRILL.md`): the real barkd, a real Ark server
      and a phone wallet. Biggest gap. Until it is done the first-run wizard
      with the real wallet daemon, the shape of real payments, and the
      on-chain rail are unconfirmed.
- [ ] **DigitalOcean on a real droplet**: paste `deploy/cloud-init.yaml`, wait,
      read the Console, finish setup in a phone browser. Note exactly what the
      Console showed and how long it took. This is the first test of the real
      internet certificate (sslip.io + Let's Encrypt).
- [ ] **WooCommerce on a real shop host** (not the CI one), with the plugin zip
      from the dry run.
- [ ] **Phone alerts** with the real ntfy app: scan the QR code, send a test.
- [ ] **Backup restore on a second machine**, from a file downloaded through the
      browser.
- [ ] **A newcomer reads `START_HERE.md`** and gets to a paid test order without
      help. Write down every place they hesitate.
- [ ] Render: only if you want it listed as available. Otherwise leave it
      "planned".

## 3. Decisions needed before the tag

- [ ] **Public name.** The GitHub account name appears in the clone address in
      the docs and `cloud-init.yaml`, in the update-check address in
      `src/updates.rs`, in `scripts/install.sh`, and in the image name. Change it
      **before** the tag (after a rename, old addresses only redirect until
      someone else registers the old name). Commits made so far also carry the
      old name inside the saved author address; hiding that needs a history
      rewrite or a fresh repository, so decide now.
- [ ] **Security reports.** `SECURITY.md` points people to GitHub's private
      reporting. Turn that on in the repository settings (Settings → Code
      security → Private vulnerability reporting), or the link leads nowhere.
      `LICENSE` is present; confirm it is the one you want.
- [ ] **Protect the `v*` tags** in the repository settings (so only you can
      create them).
- [ ] Delete stale branches, especially `mut-test` (never merge it).

## 4. Release steps (in order)

1. `main` is green and the manual **Release dry run** (Actions → release → Run
   workflow, on `main`) is green. Download the `dry-run-release-files` artifact
   and look at it: two `ebp-*.tar.gz`, the plugin zip, `SHA256SUMS`.
   The image job takes well over half an hour (arm64 under emulation).
2. Move the `Unreleased` section of `CHANGELOG.md` under a `0.1.0-rc1` heading.
3. Tag `v0.1.0-rc1` on `main` and push it. The `-rc1` suffix makes the workflow
   publish a **pre-release**, which GitHub's "latest" and the dashboard's update
   notice both ignore. Install an rc by explicit version, not `latest`.
4. Watch the release run. Then check: assets present, `SHA256SUMS` matches,
   the signature bundle exists, the notes show the image digest and the verify
   command, and the verify command works on your machine.
5. Install that release on a fresh server exactly as `START_HERE.md` says, using
   the digest-pinned image.
6. Only then tell testers. Final `v0.1.0` comes after the checklist in
   section 2 is fully ticked; the dashboard will tell rc users when it exists.

## 5. Draft release notes (edit after the drills)

> **v0.1.0-rc1 — release candidate, play money only**
>
> Easy Bitcoin Payments takes Bitcoin payments (over Ark) into a wallet you
> control, with a WooCommerce plugin and a no-terminal DigitalOcean install.
>
> **Please test with signet coins only. Do not use real bitcoin.**
>
> What works (tested automatically): setup wizard, payments (Ark, and on-chain
> when you turn it on), underpay/overpay handling, webhooks, WooCommerce
> checkout and order updates, encrypted backup and restore, upgrades that keep
> your data, phone alerts, a weekly own-wallet check, a shop-side warning if the
> payment server stops answering or orders stop being paid.
>
> Not yet verified by a person on live infrastructure: the real-wallet signet
> run, a real DigitalOcean server, phone alerts on a real phone.
>
> Not included: Shopify, Lightning payouts, refunds, automatic updates.
>
> Install: `docs/START_HERE.md` (no terminal) or `docs/START_HERE.md`.
> Verify the download with the checksums and signature in this release.
