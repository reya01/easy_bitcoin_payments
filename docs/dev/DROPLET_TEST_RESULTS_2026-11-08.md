# Droplet test, 8 November 2026 (signet, DigitalOcean SFO3)

Tester: the project owner. Result notes and the work they produced. Nothing here
was changed on the test server during the session.

## Timings

- Create Droplet to a ready server: about 15 minutes.
- Create Droplet to first successful payment (payment request only): about 25 minutes.
- WooCommerce: **not tested** (no shop was available). Still unproven on a real server.

## What worked

- Install from the setup text, wizard, admin sign-in.
- Lightning and Ark payments to a payment request.
- Pause switch: new requests refused with the plain message.
- Backup download; `ebp backup verify` opened it (9 files).

## Findings (to do after the test)

1. **Renewal loop on short-lived coins (bug).** The Ark server's coin lifetime is
   144 blocks (about 1 day) and the renewal window is 7 days, so every new coin is
   immediately "due" and is refreshed again and again. Money sits in pending
   refreshes; "Held on your server" reads 0. Evidence: `drill-capture` of that
   session (balance `pending_in_round_sat` 2828, refresh movements 2, 4, 7, 8, 9).
   Fix: refresh when blocks left <= min(first fee-tier boundary above 0%,
   one third of `vtxo_lifetime`, the lead-time setting); read the fee table
   (`fees.refresh.ppm_expiry_table`) and lifetime from barkd; fall back to the old
   rule if unreadable. Mainnet pricing quoted by the owner: refresh is 0% under
   2 days to expiry, 0.2% under 7 days, 0.4% under 14, 0.5% beyond. Add tests with
   the real numbers (144, 288) and a failing-refresh alert.
2. **Home should show arriving money** (`pending_in_round_sat`,
   `claimable_lightning_receive_sat`), not only spendable.
3. **Checkout:** show a Lightning invoice first, universal `bitcoin:` code under
   "Other ways to pay". Copy already follows the selected option (verified).
4. **Show a dollar amount next to sats** ("1,000 sats (about $0.40)") on checkout,
   admin and alerts; needs the live rate source by default and a hidden-if-missing
   fallback.
5. **Docs:** quote the payment code when pasting into a terminal (`&` and `?`);
   the backup check assumes the file is already on the server; add a "Check this
   backup" button or an upload step.
6. Backups include lock files and a debug log a restore does not need.
7. `scripts/drill_capture.sh` calls `/api/v1/exits/status`, which now redirects.

## Not yet tested on a real server

WooCommerce connect and order, owner-only pages, shop-address lock, phone alerts and
the next-day summary, expired payment page, phone scan.
