#!/usr/bin/env python3
"""Automated half of the signet drill: REAL container, REAL barkd, REAL signet
Ark server. Drives the setup wizard in a browser, then checks the things a
mock cannot: the managed-barkd handoff, barkd's real answer shapes, the
backup's coverage of barkd's real data folder, restore, and a restart.

Run from the repo root with the stack already built:  python3 tests/signet/smoke.py
Writes smoke-report.md (safe to share: no words, keys, tokens or passwords).
Payments are NOT covered (they need signet coins from the faucet).
"""
import os
import re
import subprocess
import sys
import time

from playwright.sync_api import sync_playwright

BASE = os.environ.get("EBP_PUBLIC_BASE_URL", "http://localhost:8080")
ADMIN_PW = "signet drill smoke password"
results, notes = [], []


def sh(cmd, inp=None, timeout=120):
    r = subprocess.run(cmd, shell=True, input=inp, capture_output=True, text=True, timeout=timeout)
    return r.returncode, (r.stdout + r.stderr)


def wait_for(fn, what, timeout=90, every=1.0):
    end = time.time() + timeout
    last = None
    while time.time() < end:
        try:
            v = fn()
            if v:
                return v
        except Exception as e:  # noqa: BLE001
            last = e
        time.sleep(every)
    raise AssertionError(f"timed out waiting for {what} ({last!r})")


class Abort(Exception):
    pass


def step(name, critical=False):
    def deco(fn):
        t0 = time.time()
        try:
            fn()
            results.append((name, "PASS", ""))
            print(f"PASS  {name}  ({time.time() - t0:.1f}s)", flush=True)
        except Exception as e:  # noqa: BLE001
            msg = re.sub(r"\s+", " ", repr(e))[:500]
            results.append((name, "FAIL", msg))
            print(f"FAIL  {name}: {msg}", flush=True)
            if critical:
                raise Abort(name)
        return fn

    return deco


def setup_code():
    _, out = sh("docker compose logs --no-color ebp")
    m = re.findall(r"SETUP CODE: *([A-Z0-9-]+)", out)
    return m[-1] if m else None


state = {}


def main():
    code = wait_for(setup_code, "the setup code in the server log", 180, 3)
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        ctx = browser.new_context(accept_downloads=True, viewport={"width": 1100, "height": 900})
        page = ctx.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("dialog", lambda d: d.accept())

        @step("1.1-1.2 setup page loads; wrong code refused, right code accepted", critical=True)
        def _():
            page.goto(f"{BASE}/setup")
            page.wait_for_selector("#step-code:not(.hidden)")
            page.fill("#code", "WRONG-CODE")
            page.click("#btn-code")
            page.wait_for_selector("#code-error:not(.hidden)")
            page.fill("#code", code)
            page.click("#btn-code")
            page.wait_for_selector("#step-wallet:not(.hidden)")

        @step("1.3 create the wallet on the real signet server; the server restarts itself and reconnects", critical=True)
        def _():
            page.click("#btn-wallet")
            page.wait_for_selector("#step-backup:not(.hidden)", timeout=180000)

        @step("1.4-1.5 words shown, wiped when the quiz starts; wrong quiz refused, right quiz accepted", critical=True)
        def _():
            page.check("#safe-check")
            page.click("#btn-show-words")
            wait_for(lambda: len(page.inner_text("#words").split()) >= 12, "the recovery words to appear", 120)
            words = [w for w in page.inner_text("#words").split() if w.isalpha()][-12:]
            assert len(words) == 12, len(words)
            page.check("#backup-check")
            page.click("#btn-backup")
            page.wait_for_selector("#quiz:not(.hidden)")
            assert page.inner_text("#words").strip() == "", "words must be wiped when the quiz starts"
            inputs = page.query_selector_all("#quiz-fields input")
            positions = [int(i.get_attribute("data-pos")) for i in inputs]
            for i in inputs:
                i.fill("wrongword")
            page.click("#btn-quiz")
            page.wait_for_selector("#backup-error:not(.hidden)")
            for i, pos in zip(page.query_selector_all("#quiz-fields input"), positions):
                i.fill(words[pos - 1])
            page.click("#btn-quiz")
            del words  # never kept, never printed

        @step("1.6 managed handoff needs no token copying; name the store, set the admin password", critical=True)
        def _():
            page.wait_for_selector("#step-store:not(.hidden)", timeout=60000)
            page.fill("#store", "Signet Smoke Shop")
            page.fill("#admin-pw", ADMIN_PW)
            page.click("#btn-store")
            page.wait_for_selector("#store-secrets:not(.hidden)")
            state["api_key"] = page.inner_text("#api-key").strip()
            assert state["api_key"].startswith("ebp_")
            page.click("#btn-done")
            page.wait_for_selector("#step-done:not(.hidden)")

        def login():
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#login-form:not(.hidden)")
            page.fill("#pw", ADMIN_PW)
            page.click("#login-btn")
            page.wait_for_selector("#v-home:not(.hidden)")

        @step("4.1 admin login; the checklist says the wallet is connected to the real server")
        def _():
            login()
            wait_for(lambda: "Wallet is connected" in page.inner_text("#health"), "wallet connected line", 120)
            notes.append("Home checklist:\n" + re.sub(r"\n{2,}", "\n", page.inner_text("#health"))[:1500])

        @step("2.1 a test payment request gets a real Ark address (tark1...) from barkd")
        def _():
            import json
            import urllib.request

            req = urllib.request.Request(
                f"{BASE}/api/v1/invoices",
                data=json.dumps({"order_id": "smoke-1", "sats": 1000}).encode(),
                headers={"content-type": "application/json", "authorization": f"Bearer {state['api_key']}"},
            )
            inv = json.load(urllib.request.urlopen(req, timeout=60))
            assert str(inv.get("ark_address", "")).startswith("tark1"), "no tark1 address"
            notes.append(f"Invoice created. ark_address prefix ok; lightning invoice present: {bool(inv.get('lightning_invoice'))}; onchain address present: {bool(inv.get('onchain_address'))}")

        @step("capture: barkd's real read-only answers (scripts/drill_capture.sh)")
        def _():
            rc, out = sh("./scripts/drill_capture.sh")
            assert rc == 0, out[-400:]
            files = sorted(f for f in os.listdir(".") if f.startswith("drill-capture-"))
            assert files, "no capture file"
            state["capture"] = open(files[-1]).read()
            assert "no barkd token" not in state["capture"], "barkd token could not be read inside the container"

        @step("5.5 download a backup from Settings and verify it opens with the admin password")
        def _():
            page.click("nav button[data-v=settings]")
            page.fill("#bk-admin", ADMIN_PW)
            with page.expect_download(timeout=120000) as dl:
                page.click("#bk-go")
            dl.value.save_as("smoke.ebpbackup")
            assert os.path.getsize("smoke.ebpbackup") > 1000
            assert sh("docker compose cp smoke.ebpbackup ebp:/tmp/b.ebpbackup")[0] == 0
            rc, out = sh("docker compose exec -T ebp ebp backup verify /tmp/b.ebpbackup", inp=ADMIN_PW + "\n")
            assert rc == 0, out[-400:]
            state["verify"] = out
            notes.append("Backup verify output:\n" + out[-1500:])

        @step("5.6 every barkd data file in the real folder is in the backup")
        def _():
            cap = state.get("capture", "")
            sect = cap.split("## barkd data folder", 1)[-1].split("\n## ", 1)[0]
            names = [l.split(None, 1)[1].strip() for l in sect.splitlines() if re.match(r"^\d+ ", l)]
            assert names, "no data folder listing in the capture"
            skip = ("-wal", "-shm", "-journal")
            missing = []
            for n in names:
                rel = n.split("/data/", 1)[-1]
                if rel.endswith(skip):
                    continue
                if rel not in state["verify"]:
                    missing.append(rel)
            notes.append(f"barkd data folder has {len(names)} files; not found in the backup list: {missing or 'none'}")
            assert not missing, f"missing from backup: {missing}"

        @step("5.7 restore the backup into a new folder (integrity-checked)")
        def _():
            rc, out = sh("docker compose exec -T ebp ebp backup restore /tmp/b.ebpbackup /tmp/restored", inp=ADMIN_PW + "\n")
            assert rc == 0, out[-400:]
            rc, out = sh("docker compose exec -T ebp sh -c 'find /tmp/restored -type f | wc -l'")
            notes.append(f"restored file count: {out.strip()}")

        @step("restart: the server and wallet come back by themselves, still connected, login still works")
        def _():
            assert sh("docker compose restart ebp", timeout=180)[0] == 0
            wait_for(lambda: sh(f"curl -fsS {BASE}/api/v1/health")[0] == 0, "health after restart", 150, 3)
            login()
            wait_for(lambda: "Wallet is connected" in page.inner_text("#health"), "wallet connected after restart", 150)

        @step("no page script errors")
        def _():
            assert not errors, errors[:3]

        browser.close()


try:
    main()
except Abort as a:
    print(f"ABORTED at: {a}")
finally:
    lines = ["# Signet smoke report", "", "| Step | Result | Notes |", "|---|---|---|"]
    for n, r, m in results:
        lines.append(f"| {n} | {r} | {m.replace('|', '/')} |")
    lines += ["", "## Observations", ""] + [f"```\n{n}\n```" for n in notes]
    lines += ["", "Not covered here (needs signet coins): payments, on-chain, sweeps, https cookie, phone UX."]
    open("smoke-report.md", "w").write("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if results and all(r == "PASS" for _, r, _ in results) else 1)
