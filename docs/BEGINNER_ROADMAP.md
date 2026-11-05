# Beginner & phone-first roadmap

Goal: a merchant with **no CLI experience** can install, connect and run EBP
**from a smartphone**, without weakening security. Written 2026-11-05 from a
review of the whole approach. Work proceeds in the order below.

## Where we are

Works for this audience: the setup wizard and setup code run in a browser; the
checkout page is a normal web page (hosted by EBP; the store plugin creates an
invoice through the API, sends the buyer to `/i/<id>`, and listens for the
webhook); secrets are encrypted at rest; keys are shown once; on-chain
payments wait for confirmation.

Does not work yet: **install and day-to-day management still assume a
terminal.** Specifically, setup today needs the barkd token copied out of a
container (`barkd secret show`), pasted into a `.env` file, a restart, and a
command to show the recovery words (`--expose-mnemonic`). Webhook settings,
key rotation, unmatched payments and the logs are API-only.

## Principles (do not trade away)

Keep: the one-time setup code, encryption at rest, show-once secrets, rate
limits, confirmation-gated on-chain payments, features off by default. Avoid:
emailing keys or secrets, default passwords, magic links that carry secrets,
API keys stored in the browser, unpinned auto-update.

## The plan, in order

### 1. One container + one-click deploy + HTTPS (install without a terminal)
1a. **HTTPS required for setup. (Done.)** The wizard refuses every write over
plain `http://` on a non-local address (override: `EBP_ALLOW_INSECURE_SETUP`).
1b. **Managed barkd. (Built, unverified against a live barkd.)** EBP runs barkd itself as a child process in the same
container (`EBP_MANAGED_BARKD`): it starts it, reads the token with
`barkd secret show`, applies it in memory and keeps it in the encrypted store,
and performs the recovery-words ceremony by restarting its own child with
`--expose-mnemonic` for one read. Removes the token/.env/command steps.
Binary stays checksum-pinned. The separate-container layout remains for
advanced users.
1c. **One-click deploy template. (`render.yaml` drafted, untested on the live service.)** for a host with a web dashboard (HTTPS
included, log viewer for the setup code, secret variables for
`EBP_KEY_ENCRYPTION_KEY`). Needs 1b first because such hosts run one container.

### 2. Admin page with login (manage from a phone) — BUILT
Status: built (`/admin`, also `/dashboard`). Password is chosen in the setup wizard and hashed with PBKDF2-HMAC-SHA256 (600k rounds; no new dependency). Not done: passkeys (needs a WebAuthn library), multiple admins, argon2 (a possible later swap). Original plan:

Mobile-first page for: webhook URL/secret, rotate API key, unmatched payments,
delivery log, audit log, settlement. Login by a password chosen in the wizard
(argon2 hash) and optionally a passkey; HttpOnly + SameSite=Strict session
cookie, CSRF protection, existing rate limits. No API key kept in the browser.

### 3. Alerts (nobody reads logs) — DEFERRED to beta/production releases
Decision 2026-11-05: too much setup for a beginner at this stage. Preferred design when picked up: ntfy with an auto-generated private topic, shown as a QR/link on the admin Settings tab (tap "Turn on phone alerts", install the free ntfy app, tap the link, send a test). Messages stay generic (no amounts, addresses or order numbers); the topic is stored encrypted and resettable; one alert per problem with a repeat and an "all clear". Later upgrade: Web Push straight from the admin page (no extra app; iPhone needs Add to Home Screen). Email/Telegram are harder for beginners. Limit: if the whole server is down nothing can alert; an outside uptime check would cover that. Until then the admin Home tab shows wallet health and what needs attention.

Original plan:
Push/email (ntfy or Telegram, email) for: barkd unreachable, VTXOs nearing
expiry, a payment in "unmatched", repeated webhook failures. Health indicator
at the top of the admin page.

### 4. Recovery-words gate — BUILT
Words appear only after a "I'm alone, no screenshots" checklist, for 60 seconds at a time (Hide now wipes them), and the page keeps no copy. The merchant then types three random words by number into password-style fields (no keyboard learning/suggestions); the SERVER checks them by re-reading the words from barkd for that one request (nothing kept in the browser or in server memory) and `confirm-backup` is refused until that succeeds. Remaining exposure: browser extensions/malware on the merchant's device while the words show, and the host's TLS termination in transit. Original plan:
Block going live until the merchant has recorded the words and passed a
confirmation check. (Verify what the wizard enforces today.)

### 5. "Connect my store" screen
One screen with the URL, API key and webhook secret and copy buttons, shown
once. Same exposure as today, far easier to follow on a phone.

### 6. Updates
Auto-update only from pinned, signed images; never a floating `latest`.

## Honest limit
A self-hosted server always involves some operations. For people who truly
cannot do any, the real answer is a managed hosted option, which changes the
trust model (the operator runs the server). That is a product decision, not a
code change.

## Testing scope for now
Desktop browser only. A phone-viewport pass is deferred by the maintainer.
