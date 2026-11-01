# Phase 0 — Bark Integration Spike Plan

**Status:** open
**Owner:** TBD
**Gate:** Phase 0 acceptance requires this document to be answered in writing,
with a regtest demo of receive → renew → exit and one real testnet sweep.

The spike answers five questions. Each answer must be recorded with the exact
`bark` / `barkd` version, the network, the commands run, and the observed
output. Where a command below differs from the pinned version, **the pinned
version wins** — record the real invocation in the spike log.

> **Version discipline.** Every command in this document is written against the
> Bark CLI as of the spike start. Before running anything, record
> `bark --version` and `barkd --version` in the log. If a flag has moved, fix
> the command here in the same PR that records the result. A spike answer
> without a reproducible command list is not an answer.

---

## Environment

```bash
# A throwaway regtest environment. Nothing here touches mainnet.
export BARK_NETWORK=regtest
export BARK_DATA_DIR="$PWD/.spike/bark"
export BARK_ASP_URL="http://127.0.0.1:3535"   # local ASP for regtest

mkdir -p "$BARK_DATA_DIR"

# Start the wallet daemon (headless — this is the shape EBP needs on a server).
barkd --network regtest --datadir "$BARK_DATA_DIR" --asp "$BARK_ASP_URL" &

# Confirm the daemon is up and the CLI can talk to it.
bark --network regtest --datadir "$BARK_DATA_DIR" status
```

Record: daemon start time, ASP URL, whether the ASP was local or Second's
public regtest instance, and the full `status` output.

---

## Q1 — Can a headless Bark wallet on a server reliably receive, track, renew, and exit VTXOs?

**Why it matters.** This is the whole Level 0 till. If a headless wallet cannot
renew and exit without a human, the custody ladder changes.

**How to answer it.**

```bash
# 1. Receive. Get an address and fund it from a regtest faucet / second wallet.
bark --network regtest --datadir "$BARK_DATA_DIR" receive
# → record the address, then send to it from a second regtest wallet.

# 2. Track. Confirm the VTXO appears with the right amount and expiry.
bark --network regtest --datadir "$BARK_DATA_DIR" balance
bark --network regtest --datadir "$BARK_DATA_DIR" vtxos list --json
# → record vtxo id, amount, expiry height/timestamp, and script type.

# 3. Renew. Force a renewal and confirm the VTXO id changes and the amount holds.
bark --network regtest --datadir "$BARK_DATA_DIR" vtxos renew --all
bark --network regtest --datadir "$BARK_DATA_DIR" vtxos list --json

# 4. Exit. Run a unilateral exit to a fixed on-chain address and time it.
bark --network regtest --datadir "$BARK_DATA_DIR" exit --to <regtest-btc-address> --all
# → record wall-clock duration, fee paid, and the resulting txid.
```

**Record in the log:**

- Does `vtxos list --json` expose expiry per VTXO? Field name and format.
- Does renewal require an interactive signature, or is it fully headless?
- Does `exit` complete without the ASP being online? (Stop the ASP and retry.)
- Wall-clock time and fee for a full exit of a single VTXO.
- Any state that requires a human (a prompt, a confirmation, a phone).

**Feeds:** `WalletAdapter::poll_payments`, `WalletAdapter::health`, and the
Phase 1 settlement engine's renewal/exit scheduling.

---

## Q2 — What float cap, sweep cadence, and renewal cadence keep the till safe across real fee/expiry conditions?

**Why it matters.** The Level 0 till is only honest if the cap and cadence are
derived from observed expiry windows and observed fees, not guessed.

**How to answer it.**

```bash
# Observe the VTXO lifetime the ASP actually grants on regtest.
bark --network regtest --datadir "$BARK_DATA_DIR" vtxos list --json \
  | jq '.[] | {id, amount_sats, expiry_height, expiry_at}'

# Observe renewal cost at different mempool conditions.
bark --network regtest --datadir "$BARK_DATA_DIR" vtxos renew --all --dry-run
bark --network regtest --datadir "$BARK_DATA_DIR" estimate-fee --vtxos 1
bark --network regtest --datadir "$BARK_DATA_DIR" estimate-fee --vtxos 10

# Observe exit cost — the worst case the cap must bound.
bark --network regtest --datadir "$BARK_DATA_DIR" exit --all --dry-run
```

**Record in the log:**

- Typical VTXO expiry window on regtest, and what the ASP documents for mainnet.
- Renewal fee at 1, 10, and 100 VTXOs.
- Exit fee for 1 and 10 VTXOs, at low and high mempool fee rates.
- The largest float for which a forced exit still costs < 1% of the float.

**Feeds:** the default `hot_wallet_cap_sats` (currently 500,000), the renewal
lead time (spec default: 7 days before expiry), and the sweep triggers.

---

## Q3 — What does the Boltz VHTLC swap path look like end-to-end from bark-wallet (fee profile, stuck states, failure recovery)?

**Why it matters.** Lightning receive is a v1.0 core rail. It ships with a fee
display, a trust-model doc, and a stuck-swap runbook — all of which need real
numbers and real failure modes.

**How to answer it.**

```bash
# 1. Create a reverse swap: Lightning in, Ark out.
bark --network regtest --datadir "$BARK_DATA_DIR" swap reverse \
  --amount-sats 100000 --to ark

# → record the returned invoice, the quoted swap fee, and the lockup address.

# 2. Pay the invoice from a regtest Lightning node and watch the claim.
bark --network regtest --datadir "$BARK_DATA_DIR" swap status --id <swap-id> --watch

# 3. Force the stuck path: pay the invoice, then kill the daemon before the claim.
#    Restart and confirm the swap is recovered, not lost.
bark --network regtest --datadir "$BARK_DATA_DIR" swap status --id <swap-id>
bark --network regtest --datadir "$BARK_DATA_DIR" swap recover --id <swap-id>

# 4. Force the failure path: let the swap time out without paying.
bark --network regtest --datadir "$BARK_DATA_DIR" swap status --id <swap-id>
```

**Record in the log:**

- Fee as a percentage of swap size at 10k, 100k, and 1M sats.
- Minimum and maximum swap sizes the provider accepts.
- Every terminal state the swap can reach, and which ones need a human.
- Recovery time after a daemon restart mid-claim.
- What the customer sees if the swap fails after they paid.

**Feeds:** the checkout fee display, the stuck-swap runbook, and the
`invoice.underpaid` / `invoice.expired` webhook semantics for Lightning.

---

## Q4 — What does Arké support today for receive, and what address format should sweeps target?

**Why it matters.** Arké is the reference destination wallet. Sweeps must land
somewhere the merchant can actually see on their phone, today, with no upstream
changes assumed.

**How to answer it.**

```bash
# On the server side, produce the address a sweep would target.
bark --network regtest --datadir "$BARK_DATA_DIR" receive --json
# → record the exact string format (prefix, length, checksum) and whether it is
#   an Ark address, an on-chain address, or both.

# On the phone side (manual, recorded on video):
#   1. Install Arké from the App Store / TestFlight build under test.
#   2. Create a wallet, complete the seed backup flow.
#   3. Tap Receive and record the address Arké displays.
#   4. Send a small regtest amount from the server wallet to that address.
#   5. Confirm it appears in Arké and record how long it took.
```

**Record in the log:**

- The exact address format Arké displays, and whether it matches what
  `bark receive` produces.
- Whether Arké can receive on-chain as well as Ark, and how the user chooses.
- Whether Arké exposes any signing or approval interface today (expected: no —
  record the observed answer, not the expectation).
- The seed backup flow: how many confirmations, what the user is asked to write
  down, and whether it can be completed unassisted by a non-technical tester.

**Feeds:** the sweep destination format, the setup wizard's backup step, and the
"lost phone" runbook.

---

## Q5 — What observation does the ASP support (polling endpoints, any subscription)?

**Why it matters.** The watcher is a polling adapter today. If the ASP exposes a
subscription, the adapter trait gains a second implementation; if it does not,
polling cadence and cost decide the default interval.

**How to answer it.**

```bash
# 1. Enumerate what the ASP exposes.
curl -sS "$BARK_ASP_URL/health"
curl -sS "$BARK_ASP_URL/v1/info" | jq .
# → record every endpoint, and whether any of them stream.

# 2. Check for a subscription / push surface.
curl -sS -N "$BARK_ASP_URL/v1/events" --max-time 5 || true
# → record whether this exists, what it emits, and whether it survives reconnect.

# 3. Measure polling cost.
time bark --network regtest --datadir "$BARK_DATA_DIR" vtxos list --json > /dev/null
# → run 100 times, record p50 and p95 latency and any rate limiting observed.

# 4. Measure ASP failure behaviour.
#    Stop the ASP, then poll. Record what the wallet reports and how quickly.
kill %1
bark --network regtest --datadir "$BARK_DATA_DIR" status
bark --network regtest --datadir "$BARK_DATA_DIR" balance
```

**Record in the log:**

- Every ASP endpoint, with method, auth requirement, and response shape.
- Whether a subscription exists. If yes: transport, reconnect semantics, and
  whether events are replayable from a cursor.
- Polling latency p50/p95 and any observed rate limits.
- What the wallet reports when the ASP is down, and how that maps to the
  `ok` / `degraded` / `down` traffic light on the dashboard.

**Feeds:** `WalletAdapter::poll_payments`, `WalletAdapter::health`, the default
`watcher_interval_secs`, and the ASP status indicator.

---

## Deliverables

1. **Spike log** — one section per question, each with the pinned versions, the
   exact commands, the raw output, and a one-paragraph answer.
2. **Regtest demo recording** — receive → renew → exit, end to end, no cuts.
3. **One real testnet sweep** — server till → merchant wallet, with the txid and
   the arrival confirmed in Arké.
4. **Go / no-go document** — a written confirmation (or refutation) of the
   Level 0 till design against observed behaviour, including the chosen
   `hot_wallet_cap_sats`, renewal lead time, and sweep cadence.
5. **Threat model v0** — updated with anything the spike changed.

## Out of scope for this spike

- Mainnet funds of any kind.
- Watch-only / external-signer custody (Level 1) — not assumed, not pursued.
- Lightning *send* / payouts.
- Any change to Bark or Arké upstream. EBP works with them as they exist today.