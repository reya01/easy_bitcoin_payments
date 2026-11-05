# MASTER INSTRUCTIONS — Easy Bitcoin Payments on AWS (iPhone only)

**Version: 5 — updated 2026-11-03. This file is the single source of truth for
human testers. It is updated every time a new error or fix is discovered.**

> ⚠️ EBP is EXPERIMENTAL. Signet (play-money) only. Never point it at real
> bitcoin until the EXPERIMENTAL banner in the repo README is removed.

**Goal:** set up a small cloud computer, install the EBP payment software,
and take one test payment from a phone wallet. Budget ~90 minutes.

---

## BEFORE YOU START — you need

1. An iPhone
2. This document
3. Paper and a pen (for recovery words — the most important step)
4. ~90 minutes of quiet time
5. A GitHub account (free — github.com) — needed to log in to the signet
   faucet in Part 7 (the repo itself is public; the clone needs no account)

---

## PART 1 — The cloud computer (AWS) — 15 min

1. Install **Termius** from the iPhone App Store (free). Don't open yet.
2. Safari → `https://aws.amazon.com` → **Create an AWS account** (card
   required; this test costs under $1 — stop the instance at the end).
3. Safari → `https://console.aws.amazon.com` → sign in.
4. Search bar → **EC2** → orange **Launch instance**:
   - Name: `payments`
   - Image: **Amazon Linux 2023** (default)
   - Instance type: **t4g.medium** ← IMPORTANT: the smaller t4g.small runs
     out of memory during the build (that failure = "Killed" in the logs)
   - Key pair: **Create new** → name `mykey` → RSA → .pem → Create (file
     downloads to iPhone → Files/Downloads)
   - Network settings → Edit → tick **SSH** only. Everything you use
     (dashboard, wizard, checkout) travels through the SSH tunnel from
     Part 5; leaving HTTP/HTTPS/8080 closed to the internet keeps the
     setup wizard — which shows your recovery words — unreachable publicly.
   - Storage: **16 GiB**
   - **Launch instance**
5. Tick the instance → wait for **✓ 2/2 checks passed**
6. **Write on paper: the Public IPv4 address** (e.g. `54.123.45.67`)

## PART 2 — Connect from the iPhone — 10 min

1. Files app → Downloads → hold `mykey.pem` → **Move** → **On My iPhone →
   Termius** (open Termius once first if the folder isn't there)
2. Termius → **Hosts** → **+**:
   - Address: your IPv4 from 1.6
   - Username: `ec2-user`
   - Key: **Import from Files** → `mykey.pem`
   - Save
3. ⚠️ If the instance was stopped/started at any point, its IPv4 changed —
   update the Address now.
4. Tap the host → accept the fingerprint warning → black screen with
   `[ec2-user@...]$` = you're in.
5. **To paste into the black screen: press and hold → Paste. Then tap
   Return.** Every command below is copy-paste-Return.

## PART 3 — Install (10 min)

Paste each command, wait for it to finish, then the next:

```
sudo dnf update -y
```
```
sudo dnf install -y docker git
```
```
sudo systemctl enable --now docker
```
```
sudo usermod -aG docker ec2-user
```

**Reconnect** (activates the user change): tap ✗ on the black screen, then
tap your host again.

Check:
```
docker --version
```
→ prints a Docker version line (any version number)? Continue.

**Install Docker Compose + Buildx** (two plugins Docker needs; they are NOT
included in the default install — this bit our first two testers):

```
sudo mkdir -p /usr/local/lib/docker/cli-plugins
```
```
sudo curl -L "https://github.com/docker/compose/releases/latest/download/docker-compose-linux-aarch64" -o /usr/local/lib/docker/cli-plugins/docker-compose
```
```
sudo curl -L "https://github.com/docker/buildx/releases/download/v0.28.0/buildx-v0.28.0.linux-arm64" -o /usr/local/lib/docker/cli-plugins/docker-buildx
```
```
sudo chmod +x /usr/local/lib/docker/cli-plugins/*
```

Check both:
```
docker compose version
```
```
docker buildx version
```
→ Both print a version. Continue.

## PART 4 — Get EBP and start it — 20-30 min (mostly waiting)

```
cd ~
```
```
rm -rf easy_bitcoin_payments   # FIRST SETUP ONLY — a re-run would delete
                               # your .env (the barkd token). If this server
                               # is already set up, use `git pull` instead.
```
```
git clone https://github.com/reya01/easy_bitcoin_payments.git
```
→ **No login should be requested.** If it asks for a username/token, STOP
and report — that means something changed.

```
cd ~/easy_bitcoin_payments
```
```
sudo docker compose build
```
→ **~15–30 minutes** on the first build (the barkd sidecar is a prebuilt
binary download; the EBP server itself compiles). Scrolling output is normal
— silence is what a hang looks like. If it ends with **Killed**:
out of memory — your instance is not t4g.medium; fix per Part 1.4 and redo
from `docker compose build`.

```
sudo docker compose up -d
```

Wait 60 seconds, then check:
```
sudo docker compose ps
```
→ Two services, both `Up` (one may say `health: starting` — fine).

```
curl -s http://localhost:8080/api/v1/health ; echo
```
→ **JSON status output** (any JSON = pass). If not, see TROUBLESHOOTING (bottom).

## PART 5 — Secure tunnel — 5 min

The setup wizard must only travel over an encrypted path.

1. Termius → **Port Forwarding** → **+ New / Local**:
   - Local port: `8080`, Bind: `127.0.0.1`
   - Destination: `127.0.0.1`, port: `8080`
   - Save → tap it → **Activate**
2. Keep the forwarding active whenever you use the app (wizard, dashboard,
   checkout — not just during setup).

## PART 6 — The setup wizard — 15 min

Safari → `http://localhost:8080/setup`

1. **Create till wallet** → wait ~30 s
2. Wizard shows a command → copy → paste in Termius → Return (special mode)
3. Safari → **Show my recovery words** → ✍️ write the **12 words on paper,
   numbered** — the ONLY recovery path. Never photograph, never type.
4. Tick **"I have written down my recovery words"**
5. Wizard shows a **second command** → paste in Termius → Return (special
   mode OFF again)
6. **Token step**: wizard shows a command; its output is a long code.
   Copy the long code → paste into the wizard → **Verify** → connected ✓
7. **IMPORTANT** (hands the token to the server permanently): in Termius:
   ```
   echo "EBP_BARKD_TOKEN=PASTE_THE_LONG_CODE_HERE" > ~/easy_bitcoin_payments/.env
   ```
   (replace PASTE_THE_LONG_CODE_HERE with the code — keep `EBP_BARKD_TOKEN=`
   before it), then:
   ```
   cd ~/easy_bitcoin_payments && sudo docker compose up -d
   ```
8. **Store name** → e.g. `Test Shop` → Create
9. **API key appears ONCE** → copy to paper, labelled "API key" → Finish

The wizard closes itself; the payments system is now ON.

## PART 7 — Test payment — 20 min (mostly faucet waiting)

1. Safari → `http://localhost:8080` → Share → **Add to Home Screen**
2. The dashboard is an info page only — it cannot create invoices. Create
   the test invoice with the **API key you wrote down in Part 6 step 9**:
   ```
   curl -sS -X POST http://localhost:8080/api/v1/invoices \
     -H "Authorization: Bearer PASTE-API-KEY" \
     -H "Content-Type: application/json" \
     -d '{"sats": 1000}'
   ```
   Copy the `"id"` from the reply (looks like `inv_...`) → open
   `http://localhost:8080/i/THAT-ID` in Safari → checkout page with a QR
3. Install **Arké** (iPhone) → create wallet → ✍️ write ITS 12 words on the
   paper too (label "Arké wallet")
4. Safari → `https://signet.2nd.dev/` → GitHub sign-in → paste Arké's
   **Receive** address → request sats → **wait 5–10 min** (signet is slow)
5. Arké → **Send** → scan the invoice QR → confirm 1,000 sats
6. Checkout flips to **✓ Paid** within seconds. The dashboard is an info-only
   shell in Phase 1; verify the invoice through the checkout page or the API
   (`GET /api/v1/invoices/THAT-ID` with your API key).
7. 🎉 Tester flow complete — write down: easiest step, hardest step, anything
   confusing, and whether you'd use it for a real store. Report this for the
   formal UX-gate sign-off.

## PART 8 — Cleanup — 2 min (stops billing)

**Before you stop it (optional but good hygiene): one backup snapshot —**
```
cd ~/easy_bitcoin_payments && bash scripts/backup.sh
```
Saves a consistent database snapshot + the wallet datadir (kept 14
snapshots, wallet file 600-perms) into `~/ebp-backups`. The paper recovery
words remain the only human-verifiable wallet backup; keep the paper AND the
`.env` file whenever you keep anything.

AWS → EC2 → `payments` → **Stop** (keeps disk, pennies/day) or
**Terminate** (erases everything; signet sats are play money).

---

# TROUBLESHOOTING (every error we've hit so far)

| Error | Cause | Fix |
|---|---|---|
| `Authentication failed` on git clone | repo was private (fixed — it is now public) | just retry the clone |
| `compose is not a docker command` | Compose plugin missing | Part 3, compose curl block |
| `compose build requires buildx 0.17.0 or later` | Buildx plugin missing | Part 3, buildx curl block |
| `required variable EBP_BARKD_TOKEN is missing` | HISTORICAL (pre-2026-11-02 compose) — fixed in repo | `git pull`, retry. Do NOT run `echo "EBP_BARKD_TOKEN=" > .env` on an already-set-up server — it would erase the provisioned token (re-coverable via `docker compose exec barkd barkd --datadir /data secret show`) |
| `cd: no such file or directory .../deploy` | the compose file is at the repo ROOT | `cd ~/easy_bitcoin_payments` (no /deploy) |
| Build ends with `Killed` | out of RAM (t4g.small) | use t4g.medium, rebuild |
| `could not read Username for 'https://github.com'` during build | old barkd Dockerfile cloned a nonexistent GitHub repo | fixed — `cd ~/easy_bitcoin_payments && git pull`, rebuild |
| `manifest unknown` or 404 during build | arm64 asset-name mismatch (fixed) | `git pull` and rebuild |
| `failed to solve` during build (general) | read the LAST error line above it | send the last 10 lines to the developer |
| Health curl returns nothing after `up` | build still running or service starting | `sudo docker compose ps`, wait 60 s, retry the `/api/v1/health` curl |
| Termius can't connect after restart | instance IPv4 changed on stop/start | update the host Address |
| `set: Illegal option -o pipefail` | script run with `sh` instead of `bash` | fixed in repo; use `bash scripts/...` |
| `ebp` restart-loops, `docker inspect` shows ExitCode 0, and `docker run --rm easy_bitcoin_payments-ebp --help` prints NOTHING | the image shipped the build system's placeholder binary (cargo skipped the real rebuild over stale file timestamps — fixed in the repo) | `cd ~/easy_bitcoin_payments && git pull && sudo docker compose build ebp` (must pass the build's own usage check), verify `sudo docker run --rm easy_bitcoin_payments-ebp --help` prints a usage block, then `sudo docker compose up -d` |
| barkd unhealthy / ebp waits on it | old cached barkd image | `sudo docker compose build --no-cache barkd` then `up -d` |

**Universal debugging pattern:** run the failing command again, copy the
LAST 10 lines of output, send them. Never retype an error by hand.

## Report format (send to the developer)

1. Which step number failed
2. The last 10 lines of output
3. One sentence: what you expected vs what happened
