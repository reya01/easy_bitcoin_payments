# Known issues

## OPEN — beta-hardening follow-ups (2026-11-05)

See `docs/BETA_PLAN.md` for the full ledger. Highlights an operator should know:
- The hardening branch (PR #1) was authored without a local compiler; CI (fmt, clippy -D warnings, tests, release build) is the verification. Keep it green.
- Late/unmatched payments land in the `unmatched_payments` table with no dashboard UI yet — use `GET /api/v1/unmatched-payments` (store API key) or query the table directly
- Webhooks are delivered from a durable outbox (`webhook_deliveries`); failed rows (after 6 attempts) stay for inspection with no re-send UI yet. Webhook URL validation is textual and does not stop DNS rebinding — restrict container egress in production.
- Webhook secrets are stored in plaintext (`key_encryption_key` is not implemented).
- On-chain receive detection depends on an UNPINNED `/history` shape (the deprecated `/wallet/history` is no longer used). The rail is therefore OFF by default (`EBP_ONCHAIN_RAIL_ENABLED`); do not enable it for real funds before a live signet drill.

## RESOLVED & CONFIRMED — VPS aarch64 container restart-loop (~60s, 22h duration)

**Status:** root cause proven 2026-11-03, fix committed and **confirmed on the
VPS** the same night: the tester rebuilt (`git pull` → `docker compose build`
passing the new smoke test) and ebp now boots, logs "Easy Bitcoin Payments
listening", and stays up.

**Root cause: the deployed image contained the dependency-cache STUB binary,
not ebp.** The Dockerfile's dep-cache step builds `fn main() {}` as a
placeholder; the real build replaces it. The tester's arm64 image shipped the
stub instead: `ebp --help` inside it printed NOTHING and exited 0 (a real
binary prints the USAGE block). An empty main exits instantly and cleanly —
hence `Restarting (0)` on a ~60s cadence (Docker's restart-backoff cap) for
22 hours, with no logs, OOM=false, and no change to barkd (healthy throughout
— its image downloads a prebuilt binary, so the trap can't touch it).

**Mechanism (proven locally 2026-11-03):** cargo skips relinking when source
mtimes are OLDER than the cached stub binary. git clone/pull stamps sources
with the clone time and Docker COPY preserves those old mtimes, so on every
VPS rebuild the real `cargo build` was a ~1s no-op and the stub shipped. A
`--no-cache` rebuild with the new smoke test reproduced the trap (build
failed exactly as designed); `find src -type f -exec touch {} +` before the
real build forces the recompile (proven: 7m real recompile → real 8.1MB
binary).

**Evidence chain (all 2026-11-03):**
- `docker inspect`: exit 0, OOM=false, RestartCount 1360, Started/Finished
  297ms apart — instant clean death, ~58s apart ≈ backoff cap.
- `docker events` over 26h: bare `die` events, `execDuration=0 exitCode=0`,
  ~60.3s apart, NO kill/stop before any die — the loop was self-exit +
  restart policy, not an external signaler.
- `docker run --rm easy_bitcoin_payments-ebp ebp --help` → silent, exit 0
  (stub); local build of the exact image-time commit (0e9fd88) boots and
  serves fine on x86_64 → source was never the problem.
- Exit-code matrix eliminated every earlier suspect: config/bind/db error=1,
  panic=101, OOM=137, exec failure=126/127 — none can print 0.

**History correction (recorded so it isn't repeated):** the 2026-11-02
commits (90a8a95, b264da9) attributed the loop to "crashed on 'no such
table: settings', found via tester's first-run". That was code inspection
presented as an observation — no error text was ever captured, `settings` is
read from wizard routes (not boot), and the first image was already a stub
that cannot query anything. The `"/ping never answers"` symptom was also
trivially true: EBP has no `/ping` route (the health endpoint is
`/api/v1/health`. The 0004 migration registration fix remains correct as a
real latent bug (it would have broken the setup wizard on fresh databases)
but was never the cause of the loop.

**Guards now in the Dockerfile (b0fc908, b698768):**
1. `RUN find src -type f -exec touch {} + && cargo build --release` — mtime
   refresh forces the crate recompile; dep cache untouched.
2. The dep-cache step deletes the stub binary after warming (`rm -f
   target/release/ebp`) — a skipped real build leaves no binary at all.
3. `RUN timeout 10 ./target/release/ebp --help 2>&1 | grep -qi usage` — the
   build refuses to ship anything that can't print a usage block.
4. The barkd image checksum-pins its prebuilt binary.

## Added with the open-items PR (2026-11-05)
- None of the barkd paths changed in PRs #3 and this one (Lightning invoice, Ark send, VTXO refresh, partial offboard, wallet create with `ark_server`, `/history`) has run against a live barkd; drill each on signet before real funds.
- The QR encoder is original code verified with OpenCV and one Chromium render, not a phone scan; scan a real invoice with two wallets before launch.
- Webhook secrets are still plaintext at rest (R10); history is still read in full each tick (R5).
