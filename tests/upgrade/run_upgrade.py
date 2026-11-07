#!/usr/bin/env python3
"""Upgrade + backup/restore test with two REAL binaries.

OLD = an earlier build (the "previous release"), NEW = this build. Data is
created on OLD (a paid invoice, a pending one), the server is stopped, NEW is
started on the same data folder, and everything must still be there and
working. Then an encrypted backup is downloaded from NEW, opened and restored
into an empty folder with the `ebp backup` command, and NEW is started on the
restored copy.

Usage: run_upgrade.py <old-ebp> <new-ebp>
"""
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
OLD, NEW = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
FAKE_PORT, PORT, PORT2 = 18791, 18080, 18081
TOKEN, ADMIN_PW = "up-token", "correct horse battery"
tmp = tempfile.mkdtemp(prefix="ebp-upgrade-")
procs, results = [], []


def http(method, url, body=None, headers=None):
    data = json.dumps(body).encode() if body is not None else None
    h = {"content-type": "application/json", **(headers or {})}
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def jhttp(*a, **k):
    code, hdr, raw = http(*a, **k)
    try:
        return code, hdr, json.loads(raw)
    except Exception:
        return code, hdr, raw.decode(errors="replace")


def wait_for(fn, what, timeout=40, every=0.5):
    end, last = time.time() + timeout, None
    while time.time() < end:
        try:
            last = fn()
            if last:
                return last
        except Exception as e:  # noqa: BLE001
            last = e
        time.sleep(every)
    raise AssertionError(f"timed out waiting for: {what} (last: {last})")


def step(name):
    def deco(fn):
        t0 = time.time()
        try:
            fn()
            results.append((name, True))
            print(f"PASS  {name}  ({time.time() - t0:.1f}s)", flush=True)
        except Exception as e:  # noqa: BLE001
            results.append((name, False))
            print(f"FAIL  {name}: {repr(e)[:900]}", flush=True)
        return fn

    return deco


def env_for(data, port):
    return {
        "PATH": os.environ["PATH"],
        "EBP_BIND_ADDR": f"127.0.0.1:{port}",
        "EBP_PUBLIC_BASE_URL": f"http://localhost:{port}",
        "EBP_DATABASE_PATH": os.path.join(data, "ebp.sqlite3"),
        "EBP_BARKD_URL": f"http://127.0.0.1:{FAKE_PORT}/api/v1",
        "EBP_BARKD_TOKEN": TOKEN,
        "EBP_WATCHER_INTERVAL_SECS": "1",
        "EBP_SETTLEMENT_INTERVAL_SECS": "3600",
        "EBP_WEBHOOK_ALLOW_PRIVATE": "1",
        "EBP_UPDATE_CHECK": "false",
    }


def serve(binary, data, port, name):
    f = open(os.path.join(tmp, f"{name}.log"), "w")
    p = subprocess.Popen([binary, "serve"], stdout=f, stderr=subprocess.STDOUT, env=env_for(data, port), cwd=tmp)
    procs.append(p)
    wait_for(lambda: http("GET", f"http://127.0.0.1:{port}/api/v1/health")[0] in (200, 503), f"{name} to answer")
    return p


def stop(p):
    p.terminate()
    try:
        p.wait(timeout=15)
    except subprocess.TimeoutExpired:
        p.kill()


def main():
    data = os.path.join(tmp, "data")
    os.makedirs(data)
    ff = open(os.path.join(tmp, "fake.log"), "w")
    procs.append(subprocess.Popen([sys.executable, os.path.join(ROOT, "tests", "e2e", "fake_barkd.py"), str(FAKE_PORT), TOKEN],
                                  stdout=ff, stderr=subprocess.STDOUT, cwd=tmp))
    wait_for(lambda: http("GET", f"http://127.0.0.1:{FAKE_PORT}/ping")[0] == 200, "fake barkd")
    http("POST", f"http://127.0.0.1:{FAKE_PORT}/api/v1/wallet/create", {"mnemonic": None})

    def fake(**kw):
        return jhttp("POST", f"http://127.0.0.1:{FAKE_PORT}/__ctl", kw)[2]

    base = f"http://127.0.0.1:{PORT}"
    state = {}

    # ----------------------------------------------------------------- OLD
    old = serve(OLD, data, PORT, "old")
    import sqlite3
    db = sqlite3.connect(os.path.join(data, "ebp.sqlite3"))
    db.execute("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('setup_complete','true',strftime('%s','now'))")
    db.commit()
    state["old_version"] = db.execute("PRAGMA user_version").fetchone()[0]
    db.close()
    out = subprocess.run([OLD, "store", "create", "--name", "Upgrade Shop"], capture_output=True, text=True,
                         env=env_for(data, PORT), cwd=tmp)
    assert out.returncode == 0, out.stderr
    key = re.search(r"(ebp_[A-Za-z0-9_\-]+)", out.stdout).group(1)
    auth = {"authorization": f"Bearer {key}"}

    def inv(i):
        return jhttp("GET", f"{base}/api/v1/invoices/{i}", headers=auth)[2]

    @step("OLD build: create a paid and a pending invoice")
    def _():
        c, _, a = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-a", "sats": 2000}, auth)
        assert c in (200, 201), (c, a)
        state.update(a=a["id"], a_addr=a["ark_address"])
        fake(action="pay_ark", address=a["ark_address"], sats=2000)
        wait_for(lambda: inv(a["id"])["status"] == "paid", "invoice A paid on the old build")
        c, _, b = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-b", "sats": 3000}, auth)
        assert c in (200, 201), (c, b)
        state.update(a=a["id"], b=b["id"], b_addr=b["ark_address"])

    stop(old)

    # ----------------------------------------------------------------- NEW
    new = serve(NEW, data, PORT, "new")

    @step("NEW build starts on the old data and migrates it")
    def _():
        db = sqlite3.connect(os.path.join(data, "ebp.sqlite3"))
        v = db.execute("PRAGMA user_version").fetchone()[0]
        db.close()
        assert v > state["old_version"], (state["old_version"], v)
        print(f"      schema version {state['old_version']} -> {v}")

    @step("a copy of the database was kept before the upgrade")
    def _():
        files = glob.glob(os.path.join(data, "backups", "ebp-*-before-v*.sqlite3"))
        assert files, "no pre-upgrade copy in backups/"
        c = sqlite3.connect(files[0])
        n = c.execute("SELECT COUNT(*) FROM invoices").fetchone()[0]
        c.close()
        assert n == 2, n

    @step("old invoices, the API key and idempotency survived the upgrade")
    def _():
        a, b = inv(state["a"]), inv(state["b"])
        assert a["status"] == "paid" and a["paid_sats"] == 2000, a
        assert b["status"] == "pending" and b["ark_address"] == state["b_addr"], b
        c, _, again = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-b", "sats": 3000}, auth)
        assert again["id"] == state["b"], "same order reference must return the same open invoice"

    @step("a payment for the pending old invoice is credited by the new build")
    def _():
        fake(action="pay_ark", address=state["b_addr"], sats=3000)
        wait_for(lambda: inv(state["b"])["status"] == "paid", "invoice B paid after the upgrade")

    @step("a brand-new invoice works end to end on the new build")
    def _():
        c, _, n = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-c", "sats": 1500}, auth)
        fake(action="pay_ark", address=n["ark_address"], sats=1500)
        wait_for(lambda: inv(n["id"])["status"] == "paid", "invoice C paid")

    # ------------------------------------------------------ backup / restore
    p = subprocess.run([NEW, "admin", "set-password"], input=ADMIN_PW + "\n", capture_output=True, text=True,
                       env=env_for(data, PORT), cwd=tmp)
    assert p.returncode == 0, p.stderr
    backup = os.path.join(tmp, "test.ebpbackup")

    @step("download an encrypted backup from the running server")
    def _():
        c, hdr, body = jhttp("POST", f"{base}/admin/login", {"password": ADMIN_PW})
        assert c == 200, (c, body)
        cookie = {k.lower(): v for k, v in hdr.items()}.get("set-cookie", "").split(";")[0]
        assert cookie.startswith("ebp_session="), hdr
        code, _, raw = http("POST", f"{base}/admin/backup", {"admin_password": ADMIN_PW},
                            {"cookie": cookie, "x-ebp-csrf": body["csrf"]})
        assert code == 200 and len(raw) > 200, (code, len(raw))
        open(backup, "wb").write(raw)
        assert b"SQLite format" not in raw and b"ebp_" not in raw, "backup must be encrypted"

    restored = os.path.join(tmp, "restored")

    @step("the backup opens only with the right password")
    def _():
        bad = subprocess.run([NEW, "backup", "verify", backup], input="wrong password\n", capture_output=True, text=True)
        assert bad.returncode != 0, "a wrong password must be refused"
        good = subprocess.run([NEW, "backup", "verify", backup], input=ADMIN_PW + "\n", capture_output=True, text=True)
        assert good.returncode == 0 and "ebp.sqlite3" in good.stdout, good.stdout + good.stderr

    @step("restore into an empty folder and the restored database is sound")
    def _():
        r = subprocess.run([NEW, "backup", "restore", backup, restored], input=ADMIN_PW + "\n", capture_output=True, text=True)
        assert r.returncode == 0, r.stdout + r.stderr
        assert os.path.exists(os.path.join(restored, "ebp.sqlite3"))

    stop(new)

    @step("a server started on the restored copy has all the data and the same API key")
    def _():
        s = serve(NEW, restored, PORT2, "restored")
        b2 = f"http://127.0.0.1:{PORT2}"
        for i, want in ((state["a"], "paid"), (state["b"], "paid")):
            r = jhttp("GET", f"{b2}/api/v1/invoices/{i}", headers=auth)[2]
            assert r["status"] == want, r
        stop(s)


try:
    main()
except Exception as e:  # noqa: BLE001
    print(f"FAIL  setup: {e!r}"[:1500], flush=True)
    results.append(("setup", False))
finally:
    for p in procs:
        if p.poll() is None:
            p.terminate()
    failed = [n for n, ok in results if not ok]
    if failed:
        for n in ("old", "new", "restored", "fake"):
            f = os.path.join(tmp, f"{n}.log")
            if os.path.exists(f):
                print(f"--- {n}.log (tail)\n" + open(f).read()[-2000:])
    shutil.rmtree(tmp, ignore_errors=True)

failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)}/{len(results)} steps passed")
sys.exit(1 if failed else 0)
