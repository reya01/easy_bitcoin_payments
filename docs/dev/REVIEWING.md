# Notes for reviewers and automated scanners

What the code already guards, so a review spends its time on what is left. Every statement
below names where it lives; if the code and this page ever disagree, the code wins, and please
tell us (that is a finding too). Report anything serious privately: see
[SECURITY.md](../../SECURITY.md).

## Who can do what

| Who | How they prove it | What they can do |
|---|---|---|
| Whoever runs the server | Root on the machine: the `.env` file, Docker, the database | Everything. Server configuration (`EBP_*` settings) is trusted input, not attacker input. |
| The owner, signed in | Admin session cookie plus the CSRF header on every change (`require_session`, `src/routes/admin_session.rs`) | Use the admin page. |
| The owner, for risky actions | The above, plus the admin password, plus the authenticator code or a passkey when two-step sign-in is on (`check_second_step`) | Change the payout address, start an emergency withdrawal, turn risky settings on. |
| The shop (WooCommerce plugin) | The store API key | Create and read invoices for its orders. It cannot sign in, change the payout address or move money: those routes take only the admin session. |
| Anyone | Nothing | Checkout pages (rate-limited) and `/api/v1/health`. |

## Where money can leave the server

Only two ways:

1. **Automatic transfers to the payout address** (`src/wallet/settlement.rs`). The address is
   checked again at the moment of sending: checksum, form and network (`payout::check`), and an
   Ark address on a different Ark server than the wallet's is refused
   (`payout::ark_address_on_server`). A refused address sends nothing and shows on the
   dashboard. So an address that reached the database some other way (the setup wizard's "I
   understand" path, a pending change saved before the server's network changed) still cannot receive money unless it
   passes these checks.
2. **The emergency withdrawal** (`src/routes/exit.rs`): admin session, the tick box, password
   and second step, a fresh fee estimate and fee money already on the server. Its Settings
   switch (`emergency_exit::opted_in`) is by design only a setup choice: it shows the
   withdrawal on the admin page and keeps 5,000 sats of fee money from being sent out. It is
   not a lock, and the start route does not check it; the checks above are the protection. The destination
   is saved before the wallet is asked, and later claims and left-over sends always use the
   saved one (`settled_destination`), so a later request cannot redirect it. No automatic
   transfer runs while it starts (`settlement::lock_cycle`).

## Changing the payout address

- The waiting period comes from server configuration, `EBP_PAYOUT_CHANGE_DELAY_SECS` (default
  24 hours; negative values refused at start-up, `src/config.rs`). It is never taken from a
  request: the Settings route passes `state.config.payout_change_delay_secs`
  (`src/routes/payout.rs`). `0` exists for tests (the signet drill uses it); whoever can set it
  already controls the server.
- A change needs password and second step, queues a phone notice, and can be cancelled during
  the wait.
- Only the setup wizard sets a first address at once, behind the setup code.

## Incoming payments

- Payment notices (webhooks) are only a nudge. The plugin checks the signature
  (`EBP_Core::verify_signature`), then asks the server for the invoice and acts on the server's
  answer (`handle_webhook` and `apply_invoice` in `class-ebp-gateway.php`). Nothing in the
  notice body decides whether an order is paid, so replaying one changes nothing.
- The plugin also asks the server about on-hold orders every hour for 7 days, so a lost notice
  is caught up.

## Build facts that change findings

- Release builds do not trap integer overflow (Cargo's default; `[profile.release]` in
  `Cargo.toml` does not turn it on); debug builds and fuzzing do. Money arithmetic should still
  be checked, but "panics in debug" is not "crashes the release".
- Code that is only used by tests or fuzz targets (for example the Rust
  `webhook::verify_signature`; the plugin checks signatures in PHP) is not reachable from the
  network.

## A useful report

- Names the file and line, and the path from something an outsider controls (a request, a
  payment, a notice, a file) to the problem.
- Says which of the checks above it gets past, or why they do not apply.
- Says what happens to money: lost, sent elsewhere, wrongly counted, stuck, or none.

Decided trade-offs and known limits are in [KNOWN_ISSUES.md](../KNOWN_ISSUES.md); please do
not re-report those unless something has changed.
