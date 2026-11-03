# Known issues

## Open — VPS aarch64 container restart-loop (~60s)

**Status:** open; blocks VPS sign-off.

**Environment:** tester’s AWS **t4g** instance (**aarch64**). After
`docker compose up -d`, the **ebp container restart-loops roughly every 60s**
and `/ping` never answers. Locally on **x86_64 at the same commit**, ebp runs
fine with an empty `EBP_BARKD_TOKEN` (401 warnings only, no crash). An overnight
**crashwatch log** was captured on the VPS; the next session diagnoses from that.

**Symptoms to confirm:**

- `docker compose ps` shows `ebp` restarting / unhealthy.
- `docker compose logs ebp` ends abruptly or repeats boot lines every ~60s.
- `curl http://localhost:8080/api/v1/health` or `/ping` never answers.
- `docker inspect <ebp>` shows a rising `RestartCount`; check `OOMKilled` and
  `State.ExitCode`.

**Suspected causes (not yet proven):**

1. **OOM.** The tester originally used `t4g.small`; they may still be on small.
   Rust + SQLite + barkd sidecar can exceed small-instance memory under load.
2. **ARM panic.** An aarch64-specific panic/illegal instruction in a dependency
   or in the musl/ARM build path.
3. **Pre-existing half-migrated db volume.** A volume created by an older
   schema/migration state can make startup fail before health checks answer.

**Debug procedure (do this before deleting data):**

1. Record host facts:
   ```bash
   uname -m
   free -m
   docker info --format '{{.Architecture}} {{.MemTotal}}'
   ```
2. Capture compose state and logs:
   ```bash
   docker compose ps
   docker compose logs --tail=300 ebp
   docker compose logs --tail=300 barkd
   ```
3. Capture the overnight crashwatch log (path depends on the VPS setup; check
   the tester’s notes / `~/ebp-crashwatch.log` / `journalctl`). Record the last
   exit reason and whether it repeats at a fixed interval.
4. Inspect the container:
   ```bash
   docker inspect <ebp-container> \
     --format '{{.State.ExitCode}} {{.State.OOMKilled}} {{.RestartCount}}'
   ```
5. Check kernel/memory events:
   ```bash
   dmesg -T | tail -200
   journalctl -k -n 200 --no-pager
   ```
   Look for OOM killer, segfault, illegal instruction, or cgroup memory kills.
6. Reproduce with an empty token to separate barkd/token failures from core
   startup:
   ```bash
   EBP_BARKD_TOKEN= docker compose up -d ebp
   docker compose logs --tail=200 ebp
   ```
7. Inspect the db volume without deleting it:
   ```bash
   docker volume ls
   docker compose exec ebp ls -l /data
   docker compose exec ebp sqlite3 /data/ebp.sqlite3 'PRAGMA integrity_check;'
   ```
   If the volume is half-migrated, back it up first, then test a fresh volume on
   a throwaway instance.
8. If OOM is suspected, retry on `t4g.medium` or set explicit memory limits and
   watch `docker stats`.
9. If ARM panic is suspected, capture a backtrace/debug build and record the
   exact panic line; do not assume x86_64 behavior carries over.
10. Record findings here and in [docs/HANDOFF.md](HANDOFF.md). Do **not** delete
    production data without a backup.