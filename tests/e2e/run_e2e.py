#!/usr/bin/env python3
"""Browser end-to-end test: real EBP binary + fake barkd + desktop Chromium.

Drives the whole beginner path (setup wizard with the recovery-words quiz,
admin password, login, Home checklist, Connect tab test notification) and the
money paths (Ark payment, on-chain mempool -> confirmed, RBF replacement,
barkd outage and recovery, server restart), and finally asserts that every
request EBP made to the fake matched the pinned OpenAPI spec.

Usage: run_e2e.py <path-to-ebp-binary>
"""
import hashlib
import hmac
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

from playwright.sync_api import sync_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
EBP_BIN = os.path.abspath(sys.argv[1])
FAKE_PORT, EBP_PORT, HOOK_PORT = 18791, 18080, 18999
TOKEN, SETUP_CODE, ADMIN_PW = "e2e-token", "E2E-SETUP-CODE", "correct horse battery"
BASE = f"http://localhost:{EBP_PORT}"
FAKE = f"http://127.0.0.1:{FAKE_PORT}"
WORDS = "abandon ability able about above absent absorb abstract absurd abuse access accident".split()

procs, logs = [], {}
results = []
tmp = tempfile.mkdtemp(prefix="ebp-e2e-")


def http(method, url, body=None, headers=None, timeout=10):
    data = json.dumps(body).encode() if body is not None else None
    h = {"content-type": "application/json", **(headers or {})}
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, raw.decode(errors="replace")


def ctl(**kw):
    return http("POST", f"{FAKE}/__ctl", kw)[1]


def wait_for(fn, what, timeout=25, every=0.5):
    end = time.time() + timeout
    last = None
    while time.time() < end:
        try:
            last = fn()
            if last:
                return last
        except Exception as e:  # keep polling
            last = e
        time.sleep(every)
    raise AssertionError(f"timed out waiting for: {what} (last: {last})")


def start(name, argv, env=None):
    f = open(os.path.join(tmp, f"{name}.log"), "w")
    logs[name] = f.name
    p = subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, env=env, cwd=tmp)
    procs.append(p)
    return p


def start_ebp():
    env = {
        "PATH": os.environ["PATH"],
        "EBP_BIND_ADDR": f"127.0.0.1:{EBP_PORT}",
        "EBP_PUBLIC_BASE_URL": BASE,
        "EBP_DATABASE_PATH": os.path.join(tmp, "data", "ebp.sqlite3"),
        "EBP_BARKD_URL": f"{FAKE}/api/v1",
        "EBP_BARKD_TOKEN": TOKEN,
        "EBP_WATCHER_INTERVAL_SECS": "1",
        "EBP_SETTLEMENT_INTERVAL_SECS": "3600",
        "EBP_ONCHAIN_RAIL_ENABLED": "true",
        "EBP_SETUP_TOKEN": SETUP_CODE,
        "EBP_WEBHOOK_ALLOW_PRIVATE": "1",
        "EBP_UPDATE_CHECK": "false",
        "EBP_LOG_LEVEL": "info",
    }
    p = start("ebp", [EBP_BIN, "serve"], env)
    wait_for(lambda: http("GET", f"{BASE}/api/v1/health")[0] == 200, "EBP to start")
    return p


PAGE = {"page": None, "net": []}


class Abort(Exception):
    pass


def step(name, critical=False):
    def deco(fn):
        t0 = time.time()
        try:
            fn()
            results.append((name, True, ""))
            print(f"PASS  {name}  ({time.time() - t0:.1f}s)", flush=True)
        except Exception as e:  # noqa: BLE001
            results.append((name, False, repr(e)[:600]))
            print(f"FAIL  {name}: {repr(e)[:600]}", flush=True)
            if critical:
                dump_failure_context(PAGE["page"])
                raise Abort(name)
        return fn

    return deco


# ------------------------------------------------------------ webhook sink --
received = []


class Hook(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n)
        received.append(
            {
                "body": body,
                "sig": self.headers.get("x-ebp-signature", ""),
                "event": self.headers.get("x-ebp-event", ""),
            }
        )
        self.send_response(200)
        self.end_headers()


def verify_sig(secret, item):
    parts = dict(p.strip().split("=", 1) for p in item["sig"].split(","))
    mac = hmac.new(secret.encode(), f"{parts['t']}.".encode() + item["body"], hashlib.sha256).hexdigest()
    return hmac.compare_digest(mac, parts["v1"])


def dump_failure_context(page=None):
    print("\n---- context ----")
    if page is not None:
        try:
            print("URL:", page.url)
            vis = page.evaluate(
                "Array.from(document.querySelectorAll('section')).filter(s => !s.classList.contains('hidden')).map(s => s.id)"
            )
            print("VISIBLE SECTIONS:", vis)
            print("RECENT REQUESTS:", PAGE["net"][-12:])
            print("PAGE TEXT:", page.inner_text("body")[:1500].replace("\n", " | "))
        except Exception:
            pass
    for name, path in logs.items():
        try:
            tail = open(path).read()[-2500:]
            print(f"--- {name} log (tail) ---\n{tail}")
        except Exception:
            pass


def main():
    shutil.rmtree(os.path.join(tmp, "data"), ignore_errors=True)
    os.makedirs(os.path.join(tmp, "data"))
    threading.Thread(target=HTTPServer(("127.0.0.1", HOOK_PORT), Hook).serve_forever, daemon=True).start()
    start("fake", [sys.executable, os.path.join(HERE, "fake_barkd.py"), str(FAKE_PORT), TOKEN])
    wait_for(lambda: http("GET", f"{FAKE}/ping")[0] == 200, "fake barkd")
    ebp = start_ebp()
    state = {"api_key": None, "hook_secret": None, "ebp": ebp}

    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        page = browser.new_page(viewport={"width": 1100, "height": 900})
        errors = []
        PAGE["page"] = page
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on(
            "response",
            lambda r: PAGE["net"].append(f"{r.request.method} {r.url.replace(BASE, '')} -> {r.status}")
            if "/setup" in r.url or "/admin" in r.url
            else None,
        )
        page.on("dialog", lambda d: d.accept())

        # ------------------------------------------------------- setup wizard
        @step("wizard: wrong setup code is refused, right code accepted", critical=True)
        def _():
            page.goto(f"{BASE}/setup")
            page.wait_for_selector("#step-code:not(.hidden)")
            page.fill("#code", "WRONG-CODE")
            page.click("#btn-code")
            page.wait_for_selector("#code-error:not(.hidden)")
            page.fill("#code", SETUP_CODE)
            page.click("#btn-code")
            page.wait_for_selector("#step-wallet:not(.hidden)")

        @step("wizard: create wallet, show words once, wrong quiz refused, right quiz accepted", critical=True)
        def _():
            page.click("#btn-wallet")
            page.wait_for_selector("#step-backup:not(.hidden)")
            page.check("#safe-check")
            page.click("#btn-show-words")
            page.wait_for_function("document.getElementById('words').innerText.trim().length > 20")
            shown = page.inner_text("#words").split()
            assert [w for w in shown if w.isalpha()][-12:] == WORDS or all(w in shown for w in WORDS), shown
            page.check("#backup-check")
            page.click("#btn-backup")
            page.wait_for_selector("#quiz:not(.hidden)")
            assert page.inner_text("#words").strip() == "" or "abandon" not in page.inner_text("#words"), "words must be wiped when the quiz starts"
            inputs = page.query_selector_all("#quiz-fields input")
            assert len(inputs) == 3, len(inputs)
            positions = [int(i.get_attribute("data-pos")) for i in inputs]
            for i in inputs:
                i.fill("wrongword")
            page.click("#btn-quiz")
            page.wait_for_selector("#backup-error:not(.hidden)")
            for i, pos in zip(page.query_selector_all("#quiz-fields input"), positions):
                i.fill(WORDS[pos - 1])
            page.click("#btn-quiz")
            page.wait_for_selector("#step-token:not(.hidden)")

        @step("wizard: connect token, name the store, set the admin password", critical=True)
        def _():
            page.fill("#token", TOKEN)
            page.click("#btn-token")
            page.wait_for_selector("#step-store:not(.hidden)")
            page.fill("#store", "E2E Shop")
            page.fill("#admin-pw", ADMIN_PW)
            page.click("#btn-store")
            page.wait_for_selector("#store-secrets:not(.hidden)")
            state["api_key"] = page.inner_text("#api-key").strip()
            state["hook_secret"] = page.inner_text("#webhook-secret").strip()
            assert state["api_key"].startswith("ebp_"), state["api_key"]
            page.click("#btn-done")
            page.wait_for_selector("#step-done:not(.hidden)")

        # ------------------------------------------------------ admin + shop
        @step("admin: wrong password refused, right password signs in, checklist shown")
        def _():
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#login-form:not(.hidden)")
            page.fill("#pw", "not the password")
            page.click("#login-btn")
            page.wait_for_selector("#login-err:not(.hidden)")
            page.fill("#pw", ADMIN_PW)
            page.click("#login-btn")
            page.wait_for_selector("#v-home:not(.hidden)")
            wait_for(lambda: "Wallet is connected" in page.inner_text("#health"), "wallet connected line")
            text = page.inner_text("#health")
            assert "not secure" in text, "plain http must be flagged"

        @step("connect tab: save webhook, test notification arrives with a valid signature")
        def _():
            page.click("nav button[data-v=hooks]")
            page.fill("#hook-url", f"http://127.0.0.1:{HOOK_PORT}/hook")
            page.click("#hook-save")
            page.wait_for_selector("#hook-secret-box:not(.hidden)")
            secret = page.inner_text("#hook-secret").strip()
            state["hook_secret"] = secret
            page.click("#hook-test")
            wait_for(lambda: any(r["event"] == "webhook.test" for r in received), "test webhook delivery")
            item = [r for r in received if r["event"] == "webhook.test"][-1]
            assert verify_sig(secret, item), "signature must verify with the displayed secret"

        auth = lambda: {"authorization": f"Bearer {state['api_key']}"}  # noqa: E731

        def new_invoice(order, sats):
            code, inv = http("POST", f"{BASE}/api/v1/invoices", {"order_id": order, "sats": sats}, auth())
            assert code in (200, 201), (code, inv)
            return inv

        def status_of(inv):
            return http("GET", f"{BASE}/api/v1/invoices/{inv['id']}", headers=auth())[1]

        @step("ark payment: pay the invoice's address, order flips to paid, webhook fired once")
        def _():
            inv = new_invoice("ark-1", 2500)
            assert inv["ark_address"].startswith("tark1"), inv["ark_address"]
            ctl(action="pay_ark", address=inv["ark_address"], sats=2500)
            got = wait_for(lambda: status_of(inv)["status"] == "paid" and status_of(inv), "ark invoice paid")
            assert got["paid_sats"] == 2500, got
            wait_for(lambda: sum(1 for r in received if r["event"] == "invoice.paid") >= 1, "invoice.paid webhook")
            time.sleep(2)
            ids = [json.loads(r["body"]).get("invoice_id") for r in received if r["event"] == "invoice.paid"]
            assert ids.count(inv["id"]) == 1, ids
            page.goto(f"{BASE}/i/{inv['id']}")
            wait_for(lambda: "paid" in page.inner_text("body").lower(), "checkout page shows paid")

        @step("on-chain: mempool is not credited, one confirmation credits exactly once")
        def _():
            inv = new_invoice("oc-1", 4000)
            ctl(action="pay_onchain", address=inv["onchain_address"], sats=4000, txid="a" * 64)
            time.sleep(4)
            assert status_of(inv)["status"] == "pending", "unconfirmed payment must not be credited"
            ctl(action="mine", blocks=1)
            got = wait_for(lambda: status_of(inv)["status"] == "paid" and status_of(inv), "on-chain paid after a block")
            assert got["paid_sats"] == 4000, got
            ctl(action="mine", blocks=2)
            time.sleep(3)
            assert status_of(inv)["paid_sats"] == 4000, "must not double-credit as it gains confirmations"

        @step("on-chain RBF: replaced transaction is credited once, for the mined amount")
        def _():
            inv = new_invoice("oc-rbf", 3000)
            ctl(action="pay_onchain", address=inv["onchain_address"], sats=3000, txid="b" * 64)
            time.sleep(3)
            ctl(action="pay_onchain", address=inv["onchain_address"], sats=3000, txid="c" * 64, replaces="b" * 64)
            ctl(action="mine", blocks=1)
            got = wait_for(lambda: status_of(inv)["status"] == "paid" and status_of(inv), "rbf invoice paid")
            time.sleep(3)
            assert status_of(inv)["paid_sats"] == 3000, status_of(inv)

        @step("outage: barkd down shows a red checklist and credits nothing; recovery credits the payment")
        def _():
            inv = new_invoice("down-1", 1500)
            ctl(action="set", key="down", value=True)
            ctl(action="pay_ark", address=inv["ark_address"], sats=1500)
            page.goto(f"{BASE}/admin")
            wait_for(lambda: "not answering" in page.inner_text("#health"), "red wallet line during outage")
            time.sleep(3)
            assert status_of(inv)["status"] == "pending"
            ctl(action="set", key="down", value=False)
            wait_for(lambda: status_of(inv)["status"] == "paid", "payment credited after recovery")

        @step("restart: server restarts, data and login survive")
        def _():
            inv = new_invoice("restart-1", 1200)
            state["ebp"].terminate()
            state["ebp"].wait(timeout=10)
            state["ebp"] = start_ebp()
            assert status_of(inv)["status"] == "pending"
            ctl(action="pay_ark", address=inv["ark_address"], sats=1200)
            wait_for(lambda: status_of(inv)["status"] == "paid", "paid after restart")
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#login-form:not(.hidden), #v-home:not(.hidden)")
            if page.is_visible("#login-form"):
                page.fill("#pw", ADMIN_PW)
                page.click("#login-btn")
            page.wait_for_selector("#v-home:not(.hidden)")

        @step("underpayment is not marked paid")
        def _():
            inv = new_invoice("under-1", 5000)
            ctl(action="pay_ark", address=inv["ark_address"], sats=2000)
            got = wait_for(lambda: status_of(inv)["paid_sats"] == 2000 and status_of(inv), "partial credited")
            assert got["status"] != "paid", got

        @step("every request EBP made matched the pinned barkd OpenAPI spec")
        def _():
            v = http("GET", f"{FAKE}/__ctl/violations")[1]
            assert v == [], v

        @step("no page script errors")
        def _():
            assert not errors, errors

        if any(not ok for _, ok, _ in results):
            dump_failure_context(page)
        browser.close()

    failed = [r for r in results if not r[1]]
    print(f"\n{len(results) - len(failed)}/{len(results)} steps passed")
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        try:
            rc = main()
        except Abort:
            rc = 1
    finally:
        for p in procs:
            try:
                p.terminate()
            except Exception:
                pass
    sys.exit(rc)
