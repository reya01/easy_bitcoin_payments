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
    http("POST", f"http://127.0.0.1:{FAKE_PORT}/api/v1/wallet/create", {"network": "signet"})

    def fake(**kw):
        return jhttp("POST", f"http://127.0.0.1:{FAKE_PORT}/__ctl", kw)[2]

    base = f"http://127.0.0.1:{PORT}"
    state = {}

    # ----------------------------------------------------------------- OLD
    # First start only creates the database; setup is marked complete (the
    # wizard has its own browser test) and the old build is restarted, because
    # older builds choose their wallet connection at startup.
    first = serve(OLD, data, PORT, "old-first")
    import sqlite3
    db = sqlite3.connect(os.path.join(data, "ebp.sqlite3"))
    db.execute("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('setup_complete','true',strftime('%s','now'))")
    db.commit()
    state["old_version"] = db.execute("PRAGMA user_version").fetchone()[0]
    db.close()
    stop(first)
    old = serve(OLD, data, PORT, "old")
    out = subprocess.run([OLD, "store", "create", "--name", "Upgrade Shop"], capture_output=True, text=True,
                         env=env_for(data, PORT), cwd=tmp)
    assert out.returncode == 0, out.stderr
    key = re.search(r"(ebp_[A-Za-z0-9_\-]+)", out.stdout).group(1)
    auth = {"authorization": f"Bearer {key}"}

    def inv(i):
        return jhttp("GET", f"{base}/api/v1/invoices/{i}", headers=auth)[2]

    @step("OLD build: create two invoices; one is then marked paid in its database")
    def _():
        # Older builds read payments from the wallet with a different history
        # rule than this fake wallet serves, so "paid" is recorded directly in
        # the old database. What is under test is that the data survives the
        # upgrade, not the old build's watcher.
        c, _, a = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-a", "sats": 2000}, auth)
        assert c in (200, 201), (c, a)
        c, _, b = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-b", "sats": 3000}, auth)
        assert c in (200, 201), (c, b)
        state.update(a=a["id"], b=b["id"], b_addr=b["ark_address"])

    # A build with the Invoices tab (0.3.3-rc2 and later): an invoice made on it
    # with a Lightning invoice, and a payment that arrived after it closed.
    cols = [r[1] for r in sqlite3.connect(os.path.join(data, "ebp.sqlite3")).execute("PRAGMA table_info(invoices)")]
    state["manual"] = "source" in cols and "invoice_number" in cols
    if state["manual"]:
        @step("OLD build: an invoice from the Invoices tab, its Lightning invoice and a late payment")
        def _():
            c, _, m = jhttp("POST", f"{base}/api/v1/invoices", {"order_id": "up-m", "sats": 2500}, auth)
            assert c in (200, 201), (c, m)
            state.update(m=m["id"])

    stop(old)
    dbc = sqlite3.connect(os.path.join(data, "ebp.sqlite3"))
    dbc.execute("UPDATE invoices SET status='paid', paid_sats=2000, paid_at=strftime('%s','now') WHERE id=?", (state["a"],))
    if state["manual"]:
        dbc.execute("UPDATE invoices SET source='manual', order_id=NULL, invoice_number='7', customer_name='Pat', "
                    "note='Upgrade test', status='expired' WHERE id=?", (state["m"],))
        dbc.execute("INSERT OR REPLACE INTO ln_invoices (invoice_id, bolt11, identifier, created_at) "
                    "VALUES (?, 'lntbs25u1upgradetest', NULL, strftime('%s','now'))", (state["m"],))
        dbc.execute("INSERT INTO unmatched_payments (id, address, invoice_id, rail, amount_sats, vtxo_id, reason, detected_at, created_at) "
                    "VALUES ('unm_up', 'x', ?, 'ark', 2500, 'up_late', 'paid_after_expiry', strftime('%s','now'), strftime('%s','now'))",
                    (state["m"],))
    state["count"] = dbc.execute("SELECT COUNT(*) FROM invoices").fetchone()[0]
    dbc.commit()
    dbc.close()

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
        assert n == state["count"], (n, state["count"])

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

    if state["manual"]:
        @step("the invoice from the Invoices tab survived, with its late payment shown on it and under Alerts")
        def _():
            c, hdr, body = jhttp("POST", f"{base}/admin/login", {"password": ADMIN_PW})
            assert c == 200, (c, body)
            cookie = {k.lower(): v for k, v in hdr.items()}.get("set-cookie", "").split(";")[0]
            h = {"cookie": cookie}
            c, _, lst = jhttp("GET", f"{base}/api/v1/admin/invoices", headers=h)
            assert c == 200, (c, lst)
            m = [x for x in lst if x["id"] == state["m"]]
            assert m, lst
            m = m[0]
            assert m["invoice_number"] == "7" and m["customer_name"] == "Pat" and m["status"] == "expired", m
            assert m["lightning_invoice"] == "lntbs25u1upgradetest", m
            assert m.get("late_sats") == 2500 and any(not p["counted"] for p in m.get("payments", [])), m
            c, _, alerts = jhttp("GET", f"{base}/api/v1/unmatched-payments", headers=h)
            row = [a for a in alerts if a["id"] == "unm_up"]
            assert row and row[0].get("invoice_number") == "7" and row[0].get("source") == "manual", alerts
            c, _, csv = http("GET", f"{base}/api/v1/admin/invoices.csv", headers=h)
            assert c == 200 and b"Upgrade test" in csv, (c, csv[:300])
            db = sqlite3.connect(os.path.join(data, "ebp.sqlite3"))
            have = db.execute("SELECT COUNT(*) FROM ln_invoice_history WHERE bolt11 = 'lntbs25u1upgradetest'").fetchone()[0]
            db.close()
            assert have == 1, "the old Lightning invoice is kept in the new history table"
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
