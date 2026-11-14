# UX Recommendations — Easy Bitcoin Payments

> **Scope guardrail — read first.** Everything in this document is presentation-layer work: copy, layout, disclosure, focus, display defaults, links, and documentation. Nothing here changes money movement, API contracts, custody, key handling, confirmation gates, or server behavior. Where a proper fix would need backend changes, the item says so and ships a copy-only version instead. The **Do NOT do** list at the end marks the tempting overreaches. If an item depends on data the current API doesn't return, ship the parts that don't need it and log the gap under "Maintainer follow-ups."

---

## How these conclusions were reached

**Step 1 — Repo review.** All three UIs (`assets/setup.html`, `assets/checkout.html`, `assets/dashboard.html`) and the docs (`README.md`, `docs/START_HERE.md`, `docs/WOOCOMMERCE.md`, `docs/IF_SOMETHING_LOOKS_WRONG.md`, `docs/RESTORE_DRILL.md`) were read end-to-end — markup, handlers, and copy — against two personas: a non-technical small-business operator managing from an iPhone, and an ordinary shopper with a bitcoin wallet. Findings were severity-ranked (High = task failure or money-adjacent risk; Medium = friction; Low = polish). The baseline is strong — plain-language voice, mobile fundamentals, progressive disclosure, and safety UX are already good. The failures cluster in three places: **worst-case moments** (restore, underpayment, expiry, lockout), **silence where speech is owed** (unannounced errors, stale pages, unexplained disabled buttons), and **audience mismatches** (operator messaging shown to shoppers). Findings are cited below as `SETUP-n`, `PAY-n`, `DASH-n`, `DOC-n`, `X-n`.

**Step 2 — Reference lessons.** Each finding was matched against patterns drawn from two sources: **Arké** for *register* (short declarative claims; risk disclosed to the person who can act on it, at their decision moment; trust copy as concrete negations from a named speaker; visible exits) and the **Bitcoin Design Guide** for *structure* (linear wizards; progressive security; partial payment and expiry as designed states; transaction references as support handles; validate input and confirm impactful actions; accessibility as screen skeleton; canonical units). These are cited as `A1–F2`.

**Step 3 — Synthesis.** Three rules produced this document: (1) **presentation-layer only** — anything requiring API, money-mechanics, or security-model changes was dropped or converted into a maintainer follow-up; (2) **worst-case moments first** — priority is impact-at-the-worst-moment ÷ effort; (3) **where the sources conflicted, bitcoin.design won**, because it encodes tested structure where Arké supplies tone. The specific conflicts and their resolutions:

- **Recovery-words reveal timer.** The review reads the 60-second window as avoidable stress (SETUP-13); the guide's Security guidance treats sensitive-data exposure windows as deliberate tradeoffs the product owns. *Resolution: keep 60 seconds; fix the stress with copy and a prominent re-show* (P3-S1).
- **Brevity.** Arké's ≤2-sentence cadence vs. the guide's "errors should be communicated as clearly as possible." *Resolution: Arké cadence for claims and orientation; full completeness for recovery content* — the repo's playbooks stay thorough.
- **Ledger depth.** The review wants pagination, decision records, richer history (DASH-7/DASH-8); the guide's Managing-transactions ideal needs backend support. *Resolution: display-layer handles only*; backend-dependent pieces are maintainer follow-ups, not recommendations.
- **Units.** ₿-integer display is the guide's recommended best practice with a merchant-POS precedent (Square), though the guide itself notes no community consensus. *Resolution: ₿-integer is the default recommendation; the binding rule is one canonical format everywhere* (P2-X1).
- **Custom domains.** The review suggests pushing merchants off `sslip.io` earlier (PAY-13); changing deployment defaults is out of scope. *Resolution: in-page continuity copy and docs nudges only* (P2-C6).

**The spine.** The review's four High-severity findings each had a structural principle match, and that agreement set P1:

| Finding | Fix | Traces to |
|---|---|---|
| PAY-1 underpayment dead end | P1-C2 | C4 + D1 |
| PAY-2 expired-but-payable QR | P1-C3 | C4 + D1 |
| SETUP-5 wrong-network empty wallet | P1-S2 | C2 |
| PAY-3 "Experimental" shown to shoppers | P1-C1 | A2 |
| PAY-5 amount-less on-chain QR | P1-C4 | C3 |
| SETUP-10 no Copy on one-time secrets | P1-S1 | F2 + D1 |
| DASH-1 dashboard never refreshes | P1-M3 | C1 |
| DASH-3 unguarded destructive taps | P1-M1 | D1 |
| PAY-4 no payment reference | P2-C1 | C5 |
| X-1 systematic a11y gaps | P1-X1 + P2-S2 | F1 |

The design thesis carried through every item: at the worst moment, **say the right thing to the right audience, speak when something changes, and always offer a handle.**

---

## Conventions for implementers

- **Priorities.** **P1** = high impact, low effort — do first. **P2** = high impact with more effort, or medium impact. **P3** = polish.
- **Copy is final.** Text in blockquotes is meant to ship as written; `{placeholders}` are filled at runtime. Match the repo's existing sentence-case, plain-language voice.
- **Traces.** Every item ends with the finding(s) and principle(s) it comes from.
- **Effort tags.** `[S]` ≈ an hour or less; `[M]` ≈ half a day.
- **Files in play.** `assets/setup.html`, `assets/checkout.html`, `assets/dashboard.html`, `README.md`, `docs/*.md`, plus new static assets (web manifest, icon). Nothing else.
- **Payload rule.** Use only what the current API already returns. If data is missing, ship the copy-only version and note the gap. Never add or change endpoints.
- **Suggested batching.** Four independently shippable PRs: (1) checkout P1, (2) setup P1, (3) dashboard P1, (4) docs P1. Then P2 by surface, then P3.

---

# P1 — Do these first

## P1 · Checkout (`assets/checkout.html`)

#### P1-C1 · Remove the "REAL MONEY. Experimental" banner from shopper checkout `[S]`
**Where:** the `els.banner.textContent = "REAL MONEY. Experimental: keep amounts small."` assignment in the data-load path (`test_mode === false` branch).
**Do:** Delete the assignment; render nothing on mainnet (collapse the element). Do not touch the `test_mode === true` branch — the test banner is shopper-appropriate and stays. Experimental-status honesty already lives where it belongs: the operator's dashboard and docs.
**Why:** "Keep amounts small" is advice for the person who controls the amounts and the software — the shopper controls neither. Warning styling at the conversion moment is a self-inflicted trust hit on every live order.
**Trace:** PAY-3 · A2.

#### P1-C2 · Make "underpaid" a designed top-up state, not a dead end `[S–M]`
**Where:** `poll()` underpaid branch; the pay-card render.
**Do:**
1. Replace the status text and keep the countdown visible.
2. Show the **remaining** amount everywhere the total was shown, and re-encode the on-chain QR/URI for `amount={remaining}` at the same address. If the active rail's request is a signed string whose amount can't be re-encoded client-side, hide the QR and show the address with "send exactly {remaining} more, choosing the amount in your wallet."
3. Add the overpayment guard line under the QR.
4. If the server reports the top-up window has lapsed, switch to the fallback copy.

**Copy (active top-up window):**
> **Part paid — {remaining} to go.**
> Send exactly the rest to this same code before the timer runs out, and the payment completes on its own.
>
> *Already sent some? Don't send the full amount again — that would overpay.*

**Copy (window elapsed):**
> **Part of your payment arrived, but the rest didn't come in time.**
> Contact the store and quote reference {ref} — they can refund what you sent or take it off your order.

**Verify:** the poll payload must expose the received (or remaining) amount. If it only exposes an underpaid flag, ship the copy + guard line without the figure and log the gap (see Maintainer follow-ups). Do not change the API.
**Trace:** PAY-1 · C4, D1.

#### P1-C3 · Client-side expiry guard: never leave a dead QR payable `[S]`
**Where:** countdown interval + `poll().catch(again)`.
**Do:** Track `expiresAt` from the last successful poll's countdown. On each tick: if `Date.now() > expiresAt + 5000` **and** the last successful poll is older than ~2 intervals, enter an "expired-offline" state — hide the pay card and QR, and show the copy below in the `role="status"` region. If a later poll succeeds, the server's status always wins (restore the card if still active; this covers clock skew). Also: when the server itself reports expired while reachable, hide the QR and show the same message.
**Copy:**
> **This payment request expired.**
> Don't send money to it — anything sent now won't pay this order. The store can make a new one if you still want your items.
**Trace:** PAY-2 · C4, D1.

#### P1-C4 · On-chain QR and Copy must carry the amount `[S]`
**Where:** `render()` → `drawQr(shown)` and `copyNow()` for the "Regular Bitcoin (slower)" tab.
**Do:** The wallet link already builds `bitcoin:<addr>?amount=…` — reuse that exact string: encode it in the QR, and make the primary Copy button copy it. Keep the raw address visible as text for verification, and add a small secondary "Copy address only" link (for exchanges that reject URIs). Rule: any two representations of one payment on one screen must be contract-identical (address + exact amount).
**Trace:** PAY-5 · C3.

#### P1-C5 · A "connection lost" state, so "waiting" can't lie `[S]`
**Where:** `poll().catch(again)`.
**Do:** After 2 consecutive poll failures, set the status region to the copy below; clear it when a poll succeeds. (Two failures avoids flicker on a single dropped packet.)
**Copy:**
> **Can't reach the store's server — reconnecting…**
> Check your internet connection before sending anything.
**Trace:** PAY-7 · D1.

## P1 · Setup wizard (`assets/setup.html`)

#### P1-S1 · Copy buttons on every one-time secret `[S]`
**Where:** `#store-secrets` → `#api-key`, `#webhook-secret`, and the `.cmd` docker block.
**Do:** Add the dashboard's existing `data-copy` button pattern to all three. Add one line above the secrets:
> **Shown once.** Copy each of these into your password manager now — if you leave this page, they're gone for good.
**Trace:** SETUP-10 · F2, D1.

#### P1-S2 · Restore: ask the network question in plain words, and never let an empty wallet be the last word `[S–M]`
**Where:** `#step-restore` (`#restore-net` note) and the post-restore state.
**Do:**
1. Reframe the existing network selector's copy as an explicit question (keep the underlying control and restore logic untouched):
> **Which kind of money did your old server use?**
> - Play money — it was a test server
> - Real money
>
> Not sure? Your old dashboard showed a TEST MODE banner if it was play money.
2. After a successful restore, if the balance is 0, show this directly under the balance:
> **Restored. Balance: 0.**
> If your old server used the other kind of money, your coins are safe — tap Back, pick the other one, and restore again. Your money lives in your 12 words, not in this server.
**Trace:** SETUP-5 · C2, A4.

#### P1-S3 · Live word counter on the restore textarea `[S]`
**Where:** `#restore-words`.
**Do:** On input, normalize whitespace and count words; render `{n} of 12 words` under the textarea. Block submit unless `n === 12`, with:
> That's {n} words — recovery words are always 12. Check your paper for a missed or extra word.
Optional `[M]`: validate each word client-side against the BIP39 English wordlist and highlight misses before submit.
**Trace:** SETUP-6 · C2, D1.

#### P1-S4 · Loading labels on slow wizard actions `[S]`
**Where:** `#btn-wallet`, `#btn-store`.
**Do:** Match `#btn-restore`'s pattern — disable **and** swap the label: "Creating your wallet… this can take a minute" / "Creating your store… this can take a minute."
**Trace:** SETUP-7 · D1.

#### P1-S5 · Narrate the expired-setup-code bounce `[S]`
**Where:** the `api()` 401 handler (`setup_token_required`), and `waitForRestart()`'s timeout branch.
**Do:** Before `show("code")`, set `#code-error` (which gets `role="alert"` per P1-X1) to:
> Your setup code expired — that's OK, your progress is saved. Enter the fresh code shown where your server runs (its console).
Append to the restart-timeout message: "You'll need the new setup code shown where the server runs."
**Trace:** SETUP-11, SETUP-12 · D1.

#### P1-S6 · Rewrite the backup ceremony note in the product's own voice `[S]`
**Where:** `#words-ceremony-note`.
**Do:** Replace the barkd/flag sentence with:
> Tap "Show my recovery words" to see them. We show them once, then they're gone for good — even from us. Write them on paper and keep them somewhere safe: they are the only copy of your money.
Move the mechanism into a collapsed `<details><summary>For your technical friend</summary>`:
> The words are read once from barkd's one-time `--expose-mnemonic` mode. After you confirm your backup, EBP restarts barkd without it. EBP never stores or logs the words.
**Trace:** SETUP-8 · A1.

## P1 · Dashboard (`assets/dashboard.html`)

#### P1-M1 · In-page confirmations on every destructive action `[S–M]`
**Where:** Connect tab — `#hook-rotate`, `#hook-off`; Settings — `#ex-stop`; and the existing API-key rotation's native `confirm()`.
**Do:** Build one shared in-page confirm helper (title, body, cancel label, confirm label) and use it for all four. Native `confirm()` is suppressed in some in-app browsers, which currently makes the *guarded* action silently do nothing — replace it too. Copy:
- `#hook-rotate`:
> **Make a new signing secret?**
> The current secret stops working immediately, and shops won't get payment notifications until the new secret is pasted into each shop's settings.
> [Keep current] [Make new secret]
- `#hook-off`:
> **Turn off shop notifications?**
> Stuck orders won't alert you until you turn them back on.
> [Keep on] [Turn off]
- `#ex-stop`:
> **Stop the emergency withdrawal?**
> You'll be able to start it again from the beginning.
> [Keep going] [Stop]
**Trace:** DASH-3 · D1.

#### P1-M2 · Say so when the password change succeeds `[S]`
**Where:** `$("pw-save")` success handler → `signedOut()`.
**Do:** Stash a flag, and render on the login card:
> Password changed. Sign in with your new password.
**Trace:** DASH-2 · D1.

#### P1-M3 · "Updated {time} · Refresh" + a slow poll on Home `[S–M]`
**Where:** `show()` / Home view.
**Do:** Poll the existing read endpoints every 15 s **only** while `document.visibilityState === 'visible'` and Home is the active view (pause on `visibilitychange`). Under the balance/health card, render:
> Updated 12:04 · Refresh
with Refresh as a tappable re-fetch. Add a brief CSS highlight to any row that transitions pending → paid. Never poll faster than 10 s.
**Trace:** DASH-1 · C1.

#### P1-M4 · Link every doc reference; add "Forgot your password?" at the lockout `[S]`
**Where:** all "see the docs" strings (server-error message, weekly-check "It doesn't match" flow, etc.), and `#v-login`.
**Do:** Centralize rendered-doc URLs in one `DOCS` map (GitHub rendered anchors, `target="_blank" rel="noopener"`) and convert every reference into a real link. On the login card, below the password field:
> **Forgot your password?** You can set a new one from your hosting company's Console — about 2 minutes, and the guide walks you through it. [Show me how →]({IF_SOMETHING_LOOKS_WRONG §2 anchor})
**Trace:** DASH-6, DASH-4, X-2 · D2.

#### P1-M5 · No silent failures: auxiliary cards render a retry state `[S]`
**Where:** `loadPhone()`, `loadRecon()`, `loadVersion()` — currently `.catch(function () {})`.
**Do:** On failure, render inline in the card's place:
> We couldn't check your alert settings. [Try again]
> We couldn't check your backup. [Try again]
> We couldn't check for updates. [Try again]
A missing prompt is indistinguishable from "not applicable" — a merchant who saw the alert prompt once assumes alerts are on.
**Trace:** DASH-9 · B2, D1.

## P1 · Docs

#### P1-T1 · Make the README's terminal promise honest `[S]`
**Where:** `README.md` — "**No terminal needed.**"
**Do:** Replace with:
> **No terminal for setup.** A few recovery steps use your hosting company's Console — the guide walks you through each one, copy-paste ready.
Add a short "When you'll need the Console" box to `docs/START_HERE.md` listing forgotten password · lost 2FA · software updates · the secure-address fix, each linking to its section.
**Trace:** DOC-1 · A2.

#### P1-T2 · One canonical test-mode banner `[S]`
**Where:** all three screens' test-mode strings.
**Do:** Use this exact string everywhere:
> TEST MODE: practice money only. Nothing here is real.
Operator surfaces (setup, dashboard) may append a muted second line `Network: {network}`. Identical words must mean an identical state — this is the one string a merchant must be able to trust blindly.
**Trace:** DOC-4 · E1 (consistency rule).

#### P1-T3 · Warn before the one-shot signing-secret box `[S]`
**Where:** `docs/WOOCOMMERCE.md`, immediately before the secret step.
**Do:** Add a callout:
> **Before you save:** open your WooCommerce admin in another tab now. The secret appears once, and the box disappears when you leave the page.
Do not change the plugin's one-shot behavior — see Do NOT do #1.
**Trace:** DOC-2 · D1.

## P1 · Cross-cutting

#### P1-X1 · Announce errors; kill the first-paint flash `[S]`
**Where:** every `.error` div in setup and dashboard (`#code-error`, `#wallet-error`, …); `#step-wallet` in the setup HTML source.
**Do:** Add `role="alert"` to all error containers (the checkout's `role="status"` live region is already right — copy that pattern). Add the `hidden` attribute to `#step-wallet` in the source so users requiring a code don't see "Create your wallet" flash before the correct step renders (verify `show()`/`resume()` unhides the right section).
**Trace:** X-1, SETUP-16 · F1.

---

# P2 — Schedule next

## P2 · Setup wizard

#### P2-S1 · A step indicator people (and screen readers) can read `[M]`
**Where:** `.steps` bar (`aria-hidden="true"`), `markStep(0..4)`.
**Do:** Add visible text — "Step {n} of 4 · {Step name}" with names: *Create your wallet · Back up your words · Connect your server · Create your store*. Convert the container to an `<ol>` with `aria-current="step"` on the active item (no longer `aria-hidden`). Keep the bars as visual fill if liked. Audit `markStep()` so the count matches the visible sections.
**Trace:** SETUP-1 · F1.

#### P2-S2 · One shared focus helper for every view swap `[M]`
**Where:** `show()` in setup and dashboard.
**Do:** On every section/view swap, move focus to the new section's heading:
```js
function focusHeading(section) {
  const h = section.querySelector("h1, h2, [data-focus]");
  if (!h) return;
  h.setAttribute("tabindex", "-1");
  h.focus();
}
```
This single helper lifts SETUP-16, DASH-10, and half of X-1 at once.
**Trace:** SETUP-16, DASH-10, X-1 · F1.

#### P2-S3 · Rewrite the HTTPS dead-end for the persona `[S]`
**Where:** `#step-https`.
**Do:**
> **This page needs a secure address**
> Before it can hold money, this page must load securely (https). Right now it doesn't.
> 1. Get an address with security built in — follow [the setup guide's section on addresses]({anchor in START_HERE}).
> 2. Open this page at the new address.
> 3. Pick up where you left off — your progress is saved.
Collapsed "For your technical friend": set `EBP_PUBLIC_BASE_URL` to the https:// address and reload.
**Trace:** SETUP-3 · A1, D2.

#### P2-S4 · Keep the token fallback wizard-shaped `[M]`
**Where:** `#step-token` (reached when `autoTokenTried` fails).
**Do:** Keep the command (with a Copy button per P1-S1) but reframe everything around it:
> **One step needs your server's console**
> This usually happens by itself. It didn't this time — so this step takes about a minute in the console.
> 1. Copy this command. [Copy]
> 2. Open the Console where your server runs. [How?]({link})
> 3. Paste and run it.
> 4. Paste the result below.
>
> We'll wait right here — your progress is saved.
Rewrite the success message to:
> Almost done: add `EBP_BARKD_TOKEN` next to your server's other settings and restart the payments server — [the guide shows exactly where]({link}).
**Trace:** SETUP-9 · B1, D2.

#### P2-S5 · Wrap each wizard step in a `<form>` so Go/Enter advances `[S]`
**Where:** every step's input + button (e.g., `#code` + `#btn-code`).
**Do:** Wrap in `<form>`; `submit` handler calls the existing click handler (`preventDefault`). No validation changes. The dashboard login already does this — make the wizard consistent.
**Trace:** SETUP-2 · F2.

#### P2-S6 · Hide the "don't touch this" field behind Advanced `[S]`
**Where:** `#asp`.
**Do:** Move behind `<details><summary>Advanced — Ark server</summary>` with:
> Filled in automatically for the network you chose. Change it only if you run your own Ark server. Changing the network resets this.
**Trace:** SETUP-4 · B1.

## P2 · Checkout

#### P2-C1 · Give every payment a reference both sides can cite `[S–M]`
**Where:** pay card, Help section, paid state.
**Do:** Derive a short, stable slice of the invoice id already in the URL (e.g., first 4–6 characters) and display it as `Reference {ref}` on the pay card from the start. Do **not** mint new IDs. Rewrite the Help line:
> If anything goes wrong, contact the store and quote reference {ref}.
On the paid screen: "Paid. Thank you! Keep reference {ref} in case you need to contact the store." If the payload already contains a transaction id, show it beneath the reference. The same reference must appear in the merchant's payments list (P2-M2).
**Trace:** PAY-4 · C5.

#### P2-C2 · Fix the tab pattern `[S–M]`
**Where:** `.tabs` (`aria-controls="panel"` points at nothing).
**Do:** Give each method container a real `id`, `role="tabpanel"`, and `aria-labelledby`; point `aria-controls` at it; add roving `tabindex` + arrow-key navigation. (Acceptable fallback: drop the tab roles entirely and use plain buttons with `aria-pressed` — an honest degradation beats a lying tabset.)
**Trace:** PAY-6 · F1.

#### P2-C3 · Make "Paid" grab attention `[S]`
**Where:** `#status` on paid.
**Do:** `status.scrollIntoView({ behavior: "smooth", block: "center" })`, add a brief highlight animation, and optionally `navigator.vibrate?.(200)` (guarded).
**Trace:** PAY-11 · F2.

#### P2-C4 · Move tooltip-only explanations into touch-visible text `[S]`
**Where:** `title` attributes on `#countdown`, `#sats`, the test banner, `#address`.
**Do:** The countdown gets a permanent small line:
> Bitcoin's price moves — this price is held for the time shown.
Ensure every copy affordance has a visible label ("Copy"), keeping `title` only as a bonus. `title` tooltips don't exist on touch, which is this page's primary modality.
**Trace:** PAY-9 · F2.

#### P2-C5 · Clipboard fallback `[S]`
**Where:** `copyNow()`'s swallowed failure handler.
**Do:** On failure, select the string and show:
> Copy didn't work — press and hold the address, then choose Copy.
**Trace:** PAY-10 · F2.

#### P2-C6 · Bridge the domain trust gap; name the speaker `[S–M]`
**Where:** header area and footer.
**Do:** Under the store name, add a continuity line: "You are paying {store name} · Reference {ref}". Rewrite the footer:
> Bitcoin payments cannot be reversed, so refunds are up to the store. This page is {store}'s own bitcoin till, powered by Easy Bitcoin Payments — no account, no email, no tracking.
In docs and the setup done screen, add a one-line nudge toward a custom domain ("Looks better on your own domain — see 'Later, when you're ready' in the guide"). Copy only — do not change deployment defaults.
**Trace:** PAY-13, PAY-14 · A3.

## P2 · Dashboard

#### P2-M1 · Give Settings an anchor nav `[M]`
**Where:** `#v-set` — eleven `<h2>` sections in one scroll.
**Do:** Add sticky "chips" at the top that scroll to section anchors. Suggested grouping (DOM reorder optional; anchors alone fix findability): **Money** (payout, cap, sweep, emergency withdrawal) · **Security** (password, 2FA) · **Alerts** (pause, phone alerts) · **Upkeep** (backup, check-backup, audit log). Keep the emergency withdrawal folded at the end of Money — that placement is correct.
**Trace:** DASH-5 · B1.

#### P2-M2 · Make the payments list able to support support `[M]`
**Where:** `loadOrders()`.
**Do:** Add the reference column (same `{ref}` as checkout, monospace, `font-variant-numeric: tabular-nums`), relative times ("2 hours ago", absolute in `title`), and — if the invoice URL is derivable from the id using the checkout page's existing `/i/…` route — make the row open it. Do not add pagination (needs API params; see Do NOT do #8).
**Trace:** DASH-7 · C5, E2.

#### P2-M3 · Tell the merchant how to handle an unmatched payment `[S]`
**Where:** `#v-alerts` unmatched card.
**Do:** Add per-rail guidance and a caption on the button:
> **How to refund this**
> - Lightning: there's no return address on Lightning. Ask the customer for a Lightning invoice, or send from your own wallet.
> - On-chain: this can usually go back to the address it came from — check the transaction in your wallet.
>
> "Handled" means you've decided what to do — fulfilled or refunded. The payment stays in your history.
Link the relevant playbook section inline.
**Trace:** DASH-8 · C5, D2.

#### P2-M4 · The emergency-withdrawal button narrates its own preconditions `[S–M]`
**Where:** `exReady()` / `#ex-start`.
**Do:** Derive from the same conditions `exReady()` already evaluates and render a live line above the button, e.g.:
> Before you can start: two-step sign-in on · your deposit confirmed · the checkbox above
Dynamic variant while waiting on the deposit: "Waiting for your deposit to confirm — checked again in {n}s." Do not change the gating logic.
**Trace:** DASH-12 · A4.

#### P2-M5 · Accessible active state on the bottom nav `[S]`
**Where:** `nav#tabs`.
**Do:** Set `aria-current="page"` on the active tab button; focus management comes free from P2-S2.
**Trace:** DASH-10 · F1.

#### P2-M6 · Wire the dashboard as an installable app (manifest + meta only) `[S–M]`
**Where:** `dashboard.html` `<head>`; new static assets.
**Do:** Add `manifest.webmanifest` (name "Easy Bitcoin Payments", short_name, `display: "standalone"`, `start_url` = the dashboard's path, theme/background colors matching the existing palette, one SVG icon), `<meta name="theme-color">` for light and dark, `apple-mobile-web-app-capable`, and an `apple-touch-icon`. **No service worker** (see Do NOT do #7).
**Trace:** DASH-11 · F2.

## P2 · Cross-cutting

#### P2-X1 · Pick one canonical amount format and sweep everything `[M]`
**Where:** all three screens + docs.
**Do:** Default recommendation (per the guide's ₿-only format, BIP-177, with the Square merchant-POS precedent): integer satoshis with the ₿ symbol — `₿5,449` — with fiat always adjacent beneath or beside ("$0.58"). Rules: thin-space (U+2009) grouping; `~` prefix for rounded small amounts; tabular or monospace figures in the payments list with consistent decimals; a slashed-zero or 0/O-safe rendering for addresses; keep `overflow-wrap: anywhere`. If maintainers prefer keeping "sats," that is acceptable — the binding rule is **one format everywhere** (the DOC-3/DOC-4 drift is the sin, not the choice).
**Trace:** E1, E2 · DOC-3/DOC-4 precedent.

#### P2-X2 · One noun for the thing being paid `[S]`
**Where:** setup store step ("invoices"), checkout ("payment request"), dashboard ("payments"), docs ("invoice").
**Do:** Canonical user-facing noun: **payment**. Keep "invoice" only in API/technical docs. Sweep all four surfaces.
**Trace:** DOC-3.

---

# P3 — Polish

#### P3-S1 · Reassure around the 60-second reveal `[S]`
Under the reveal timer: "Take your time — if it runs out, you can show the words again before you finish." Keep the 60-second window itself (see conflict resolution on the timer). **Trace:** SETUP-13.

#### P3-S2 · Quiz inputs: stop iOS fighting the user `[S]`
Add `autocomplete="off" autocapitalize="none" spellcheck="false"` to the three quiz inputs; keep `type="password"` (the shoulder-surfing rationale stands). **Trace:** SETUP-14.

#### P3-S3 · Store step: explain the thing being asked `[S]`
Replace the "authenticates as… invoices" sub-copy with: "Name this till so you recognize it later. Shoppers don't see this name." **Trace:** SETUP-15 · A1.

#### P3-C1 · Never leave "Loading…" after a failed load `[S]`
On checkout load failure, clear `#store-name` (or set it to "This store") so the error message doesn't sit under a half-broken header. **Trace:** PAY-8.

#### P3-C2 · Scroll the QR into view when the method changes `[S]`
On tab change, if the QR is out of the viewport, `scrollIntoView`; or render an inline "Now paying with {method}" chip next to the tabs. **Trace:** PAY-12.

#### P3-M1 · Mode-aware "Try a payment" copy `[S]`
Test mode: "This server runs on play money — send play-money coins." Mainnet: "This server runs on real money — try a small amount from your own wallet first." **Trace:** DASH-13.

#### P3-M2 · Feedback and consistency cluster `[M]`
- Relative times everywhere via one helper ("just now · 5 min ago · 2 hours ago · yesterday"), absolute in `title` (reuse in the payments list and audit log).
- Chevron/underline affordance on tappable "Needs attention" rows; make the visual match the behavior.
- Subtle loading placeholders so lists don't flash empty.
- 2FA recovery codes: add "Download" (Blob → .txt) and "Print" (print stylesheet) alongside Copy.
- Session expiry message: "Your session expired — please sign in again."
- Audit log: map known action names to plain labels ("payout change requested" → "Payout address change requested"), raw value in `title`.
- Alerts badge: if the attention payload already includes webhook failures, count them; otherwise leave as-is.
**Trace:** DASH-14 · E2, C5.

---

## Maintainer follow-ups (out of scope for this pass — worth proposing, not implementing)

- Persist the WooCommerce signing-secret box until first successful delivery (or explicit dismiss) — would delete the docs' most-used troubleshooting section. Requires plugin/server change.
- Expose remaining/received amount (and txid once paid) in the checkout poll payload, if absent — unlocks the full P1-C2 state.
- Payments-list pagination and per-payment notes/decision records (DASH-7/DASH-8).
- A guided in-product restore drill with the answer key inline (RESTORE_DRILL as a flow, not a doc).
- A custom-domain walkthrough earlier in onboarding docs (the root cause of PAY-13).

## Definition of done

- [ ] Every changed screen exercised on iPhone Safari (primary) and Android Chrome.
- [ ] VoiceOver pass: wizard announces "Step {n} of 4" and errors; checkout tabs operable; dashboard nav announces the active tab.
- [ ] Keyboard-only: setup and dashboard fully operable; focus lands on section headings after every swap.
- [ ] `git diff` touches only `assets/*.html`, `docs/*.md`, `README.md`, and new static assets (manifest, icon).
- [ ] Jargon pass: `docker`, `exec`, `EBP_`, `--flag`, `BIP` appear only inside "For your technical friend" disclosures or technical docs.
- [ ] String sweep: one test-mode banner, one noun for "payment," one amount format across all surfaces.

---

## Do NOT do — tempting changes that cross the line

1. **Do not persist, re-show, or log one-time secrets.** The fix for SETUP-10/DOC-2 is Copy buttons and warnings — not making the API key or webhook secret viewable twice. "Shown once" is the security model.
2. **Do not lengthen or remove the 60-second recovery-words reveal.** It is a deliberate exposure window (see the conflict resolution on the timer). Copy reassurance only.
3. **Do not touch money mechanics.** Top-up window length, invoice expiry, the payout-address day delay, caps, sweep and emergency-withdrawal rules stay exactly as they are. These screens may *display* those rules; they may never alter them.
4. **Do not change what a QR points at, and do not add shopper-editable amounts.** P1-C4 adds the *existing expected amount* to the *existing address*. The guide's "users can change units" means display units — on checkout, the amount is the payment contract.
5. **Do not auto-detect or auto-switch networks on restore.** The wrong-network fix is a question before and a warning after (P1-S2). Chain-scanning or silently retrying on the other network changes restore semantics.
6. **Do not remove, weaken, or auto-confirm safety gates.** The mainnet checkbox, the 2FA skip-confirmation, the payout-change delay, and the emergency-withdrawal preconditions get *narrated* (P2-M4), never bypassed. Every confirm added in P1-M1 must require a deliberate tap.
7. **Do not add a service worker or any offline cache for payment pages.** A stale cached checkout showing a payable QR is the exact failure P1-C3 exists to prevent. Manifest + meta only.
8. **Do not add or change API endpoints, parameters, or payloads.** No pagination params, no txid field, no handled-decision notes, no received-amount field. If a payload lacks data an item needs, ship the copy-only version and log it under Maintainer follow-ups.
9. **Do not change the `sslip.io` default domain or auto-provision domains.** Continuity copy and docs nudges only (P2-C6).
10. **Do not record what "Mark as handled" decided, or add merchant notes to payments.** Needs backend storage; out of scope.
11. **Do not poll aggressively.** Dashboard polling stays at 10–15 s minimum and pauses when the tab is hidden. The checkout's 1-second poll is the shopper's money moment; the dashboard's is not.
12. **Do not "simplify" the new confirmations back into native `confirm()`.** Native dialogs are suppressed in some in-app browsers — that suppressibility is why P1-M1 builds an in-page pattern.
