# Design note: the payment watcher re-reads the whole history (planning only)

**Status:** proposal, no code. Not for the first release candidate. Decide after
the signet drill shows whether it matters in practice.

## What happens today

Every 10 seconds the watcher asks the wallet (barkd) for its payment history.
The barkd call ignores the `since` value we pass (`payments_since` in
`src/wallet/barkd.rs`), so we get the **whole history every time**. For each
entry the watcher then runs two small database lookups
(`find_pending_by_address`, and `record_unmatched` for strangers).

Nothing is wrong or unsafe about this: repeated entries are no-ops because the
database refuses duplicates (unique VTXO/txid guards), and the cursor only
moves forward. The cost is that **work grows with the lifetime number of
payments**, not with new ones. A shop with a few hundred payments will never
notice; a busy shop after months might (more CPU, more SQLite reads, a larger
response from barkd every 10 seconds).

Migration 0010 (separate branch) already indexes the invoice lookup, which
removes the worst of the per-entry cost. This note is about the rest.

## What we do not know yet

1. How big the barkd history response gets and how long it takes (measure in the
   signet drill with a few hundred tiny payments; log tick duration).
2. Whether barkd offers any "since", page or limit option in 0.8.x that we are
   not using. Check its API reference before building anything.
3. Whether re-delivering an entry can legitimately **change** something, for
   example an unconfirmed entry later gaining `confirmed_at`. If it can, those
   entries must keep being re-read until they settle. Read `record_payment` in
   `src/invoice.rs` and the barkd entry shape to answer this.

Do not start until 1 to 3 are answered.

## Options

**A. Remember what is already handled (in memory).** After an entry has been
recorded (payment applied, late payment recorded, or parked as unmatched),
remember its key (VTXO id or txid plus address) in a set and skip it on later
ticks. The set is rebuilt after a restart by one full pass.
- Pro: removes almost all database work. Simple to reason about.
- Con: memory grows with history (a key is about 100 bytes: 10,000 payments is
  about 1 MB, fine). Only valid if question 3 is "no".
- Safety rule: only remember an entry **after** the database write succeeded. A
  failed write must be retried on the next tick.

**B. Only look at recent entries.** Skip entries older than the cursor minus a
safety window (for example one hour), because they were handled long ago.
- Pro: no growing set; tiny code change.
- Con: relies on the wallet's timestamps being sane. A payment the wallet
  reports late with an old timestamp could be skipped.
- Mitigation: also run a full pass every N ticks (for example hourly) as a
  safety net. Duplicates are harmless, so the full pass is always correct.

**C. Ask barkd for less.** Only possible if question 2 turns up a real option.
Best result, but we depend on a barkd feature.

## Recommendation

Prefer **B with a periodic full pass** if question 3 is "yes, entries can
change", or **A** if "no". Either way: keep the existing database guards as the
final defence, make the change behind a setting that can fall back to
"always read everything", and add a test that proves a payment which fails to
record on tick 1 is picked up on tick 2.

## Test plan (when we build it)

- Unit: a failed write is not remembered and is retried.
- Unit: an old entry is skipped, a new entry is applied, and the hourly full pass
  still catches an old-timestamp entry.
- Signet drill: 300 small payments, then check tick duration and CPU stay flat.
- No change to sweep or settlement logic.
