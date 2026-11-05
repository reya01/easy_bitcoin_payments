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
- Payments: an Ark payment (paid once, one notification), an on-chain payment
  (not credited in the mempool, credited once after a block, never twice), an
  on-chain payment replaced by a higher-fee one (credited once), an
  underpayment (not marked paid).
- Failures: barkd down (red checklist, nothing credited, credited after it
  returns) and a server restart (data and the signed-in session survive).

The fake barkd (`tests/e2e/fake_barkd.py`) checks every request EBP sends and
every response it returns against the pinned OpenAPI spec
(`docs/barkd-openapi.json`) and fails the run on any mismatch, so it cannot
drift from the documented API.

What it does NOT prove: that the real barkd behaves like its OpenAPI spec, the
managed-barkd process handoff, real money, HTTPS termination, or phones. Those
are what `docs/SIGNET_DRILL.md` is for.

Run it yourself (needs Python 3, `pip install playwright jsonschema`,
`playwright install chromium`, and a built binary):

    cargo build --release
    python3 tests/e2e/run_e2e.py target/release/ebp
