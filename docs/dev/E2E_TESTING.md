# Browser end-to-end test

Runs on every CI build (`tests/e2e/`). It starts the real `ebp` binary, a fake
barkd, and desktop Chromium, then walks the whole beginner path and the money
paths. It takes about a minute.

What it covers:

- Setup wizard: wrong then right setup code, wallet creation, recovery words
  shown, wrong then right quiz answers (checked by the server), token step,
  store name and admin password.
- Admin page: wrong then right password, the "Is everything OK?" checklist,
  saving a shop notification address, and a signed test notification that the
  test receiver verifies with the displayed secret.
- Payments: an Ark payment (paid once, one notification), the check that no
  invoice or checkout answer carries an on-chain address or a combined code, an
  underpayment (not marked paid).
- Failures: barkd down (red checklist, nothing credited, credited after it
  returns) and a server restart (data and the signed-in session survive).

Besides this walk-through, CI runs smaller browser tests that load one page with
pretend answers from the server (no real server needed): `checkout_states.py`
(waiting, paid, expired, the Ark box), `setup_wizard_states.py`, `dashboard_states.py`
(Home, Settings and Alerts states, including the send-minimum and kept-amount boxes,
the emergency withdrawal on and off, and the send-back button), `mainnet_screens.py`
(the real-money wording), `update_card.py` (the Home update card), `page_assets.py`
and `startup_errors.py`. They run at phone size and fail on anything the page's
Content-Security-Policy blocks. The pretend answers are written by hand, so when a
route's answer changes, change the matching test page too.

The fake barkd (`tests/e2e/fake_barkd.py`) checks every request EBP sends and
every response it returns against the pinned OpenAPI spec
(`docs/barkd-openapi.json`) and fails the run on any mismatch, so it cannot
drift from the documented API.

What it does NOT prove: that the real barkd behaves like its OpenAPI spec, the
managed-barkd process handoff, real money, HTTPS termination, or phones. Those
are what `docs/dev/SIGNET_DRILL.md` is for.

Run it yourself (needs Python 3, `pip install playwright jsonschema`,
`playwright install chromium`, and a built binary):

    cargo build --release
    python3 tests/e2e/run_e2e.py target/release/ebp
