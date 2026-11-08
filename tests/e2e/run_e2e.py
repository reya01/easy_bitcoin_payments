#!/usr/bin/env python3
"""Browser end-to-end test: real EBP binary + fake barkd + desktop Chromium.

Drives the whole beginner path (setup wizard with the recovery-words quiz,
admin password, login, Home checklist, Connect tab test notification) and the
money paths (Ark payment, on-chain mempool -> confirmed, RBF replacement,
barkd outage and recovery, server restart, encrypted backup download and restore), and finally asserts that every
request EBP made to the fake matched the pinned OpenAPI spec.

Usage: run_e2e.py <path-to-ebp-binary>
"""
import base64
import hashlib
import hmac
import struct
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
        # Lets a step pretend to be a different visitor (X-Forwarded-For), so the
        # sign-in brake (10 tries a minute per visitor) does not trip a long run.
        "EBP_TRUST_PROXY_HEADERS": "true",
        "EBP_UPDATE_CHECK": "false",
        "EBP_LOG_LEVEL": "info",
    }
    p = start("ebp", [EBP_BIN, "serve"], env)
    wait_for(lambda: http("GET", f"{BASE}/api/v1/health")[0] == 200, "EBP to start")
    return p


PAGE = {"page": None, "net": []}

# Phone mode: EBP_E2E_DEVICE="iPhone 13" runs the whole flow in an emulated
# phone browser (touch, small screen) and, after every step, checks that the
# page does not scroll sideways. Tap-target and text-size findings are printed
# as PHONE NOTE lines; screenshots go to $EBP_E2E_SHOTS when set.
DEVICE = os.environ.get("EBP_E2E_DEVICE", "")
SHOTS = os.environ.get("EBP_E2E_SHOTS", "")
LAYOUT_JS = r"""() => {
  const vw = window.innerWidth;
  const over = document.documentElement.scrollWidth - vw;
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const cs = getComputedStyle(el);
    return cs.visibility !== 'hidden' && cs.display !== 'none' && el.type !== 'hidden';
  };
  const label = (el) => ((el.id ? '#' + el.id : '') + ' ' + (el.textContent || el.value || el.tagName)).trim().replace(/\s+/g, ' ').slice(0, 32);
  const small = [], zoom = [], wide = [];
  for (const el of document.querySelectorAll('button, a[href], input:not([type=checkbox]):not([type=radio]), select, textarea, summary')) {
    if (!visible(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.height < 36) small.push(label(el) + ' ' + Math.round(r.width) + 'x' + Math.round(r.height));
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName) && parseFloat(getComputedStyle(el).fontSize) < 16) zoom.push(label(el));
  }
  for (const el of document.querySelectorAll('body *')) {
    if (!visible(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.right > vw + 1) wide.push(label(el) + ' right=' + Math.round(r.right));
  }
  return {over: Math.round(over), vw, small: small.slice(0, 8), zoom: zoom.slice(0, 8), wide: wide.slice(0, 6)};
}"""


def phone_audit(label):
    """Returns an error string for hard problems, else ''. Notes are printed."""
    page = PAGE["page"]
    if not DEVICE or page is None:
        return ""
    try:
        m = page.evaluate(LAYOUT_JS)
        if SHOTS:
            os.makedirs(SHOTS, exist_ok=True)
            safe = "".join(c if c.isalnum() else "-" for c in label.lower())[:60].strip("-")
            page.screenshot(path=os.path.join(SHOTS, f"{len(os.listdir(SHOTS)):02d}-{safe}.png"), full_page=True)
    except Exception as e:  # noqa: BLE001
        print(f"PHONE NOTE  {label}: audit skipped ({e!r})"[:300], flush=True)
        return ""
    for key in ("small", "zoom"):
        if m[key]:
            print(f"PHONE NOTE  {label}: {key}: {m[key]}", flush=True)
    if m["over"] > 1:
        return f"page scrolls sideways by {m['over']}px on a {m['vw']}px screen: {m['wide']}"
    return ""


class Abort(Exception):
    pass


def step(name, critical=False):
    def deco(fn):
        t0 = time.time()
        try:
            fn()
            bad = phone_audit(name)
            assert not bad, bad
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
HOOK = {"fail": False, "failed_bodies": []}


class Hook(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n)
        if HOOK["fail"]:
            # The shop is "down": refuse, and remember we were asked.
            HOOK["failed_bodies"].append(body)
            self.send_response(503)
            self.end_headers()
            return
        received.append(
            {
                "body": body,
                "sig": self.headers.get("x-ebp-signature", ""),
                "event": self.headers.get("x-ebp-event", ""),
            }
        )
        self.send_response(200)
        self.end_headers()


def totp_code(secret_b32, step):
    """The 6-digit code an authenticator app shows (RFC 6238, SHA-1, 30 s)."""
    key = base64.b32decode(secret_b32.upper() + "=" * (-len(secret_b32) % 8))
    mac = hmac.new(key, struct.pack(">Q", step), "sha1").digest()
    off = mac[-1] & 15
    return f"{(struct.unpack('>I', mac[off:off + 4])[0] & 0x7FFFFFFF) % 1000000:06d}"


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
        if DEVICE:
            page = browser.new_context(**pw.devices[DEVICE]).new_page()
        else:
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
            wait_for(lambda: len(page.inner_text("#words").strip()) > 20, "recovery words to appear")
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
            page.wait_for_selector("#store-error:not(.hidden)")
            assert "Choose where your money" in page.inner_text("#store-error")
            page.select_option("#payout-kind", "ark")
            page.click("#btn-store")
            assert "Paste your wallet" in page.inner_text("#store-error")
            page.fill("#payout", "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx")
            page.click("#btn-store")
            assert "does not look like an Ark address" in page.inner_text("#store-error")
            page.select_option("#payout-kind", "onchain")
            page.fill("#payout", "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh")
            page.click("#btn-store")
            assert "looks like an Ark address" in page.inner_text("#store-error")
            page.select_option("#payout-kind", "ark")
            before = page.inner_text("#store-error")
            page.fill("#payout", "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjX")
            page.click("#btn-store")
            wait_for(lambda: page.inner_text("#store-error") != before, "server rejects a bad address")
            err = page.inner_text("#store-error")
            assert "[object" not in err and len(err) > 10, err
            page.fill("#payout", "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh")
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
            assert "only works on this computer" in text, "a localhost address must be flagged"

        @step("two-step sign-in: offered with a warning to skip, turned on with an app code, asked at sign-in, turned off")
        def _():
            page.context.set_extra_http_headers({"x-forwarded-for": "10.77.0.1"})
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#v-home:not(.hidden)")
            wait_for(lambda: page.is_visible("#twofa-prompt"), "the offer to turn on two-step sign-in")
            page.get_by_role("button", name="Skip, use password only").click()
            wait_for(lambda: "less secure" in page.inner_text("#twofa-prompt"), "warning about password-only")
            page.get_by_role("button", name="Turn on instead (recommended)").click()
            page.wait_for_selector("#tf-pw")
            page.fill("#tf-pw", ADMIN_PW)
            page.get_by_role("button", name="Continue").click()
            page.wait_for_selector("#tf-key")
            secret = page.inner_text("#tf-key").replace(" ", "").strip()
            assert len(secret) >= 32, secret
            assert page.is_visible("#twofa-card img"), "a QR picture is shown"
            if DEVICE:
                problem = phone_audit("two-step setup")
                assert not problem, problem
            cur = int(time.time()) // 30
            page.fill("#tf-code", "000000")
            page.get_by_role("button", name="Turn on two-step sign-in").click()
            wait_for(lambda: page.is_visible("#tf-msg"), "wrong code refused")
            page.fill("#tf-code", totp_code(secret, cur))
            page.get_by_role("button", name="Turn on two-step sign-in").click()
            page.wait_for_selector("#tf-codes")
            codes = [c for c in page.inner_text("#tf-codes").split() if c]
            assert len(codes) == 10, codes
            page.get_by_role("button", name="I saved them").click()
            wait_for(lambda: "Backup codes left: 10" in page.inner_text("#twofa-card"), "turned on, 10 backup codes")
            assert page.is_visible("#bk-code"), "protected actions now ask for a code"
            # Sign out and back in: the password alone is no longer enough.
            page.click("#logout")
            page.wait_for_selector("#login-form:not(.hidden)")
            page.fill("#pw", ADMIN_PW)
            page.click("#login-btn")
            page.wait_for_selector("#code-box:not(.hidden)")
            page.fill("#code", "000000")
            page.click("#login-btn")
            wait_for(lambda: "code" in page.inner_text("#login-err"), "wrong code refused at sign-in")
            page.fill("#code", totp_code(secret, cur + 1))
            page.click("#login-btn")
            page.wait_for_selector("#v-home:not(.hidden)")
            assert not page.is_visible("#twofa-prompt"), "no offer once it is on"
            # A backup made while two-step is on must not carry its secret or
            # backup codes (the password alone opens the file).
            page.click('#tabs button[data-v="set"]')
            page.wait_for_selector("#bk-go")
            page.fill("#bk-admin", ADMIN_PW)
            page.fill("#bk-code", totp_code(secret, cur + 1))
            with page.expect_download(timeout=60000) as dl2:
                page.click("#bk-go")
            bpath = os.path.join(tmp, "backup-2fa.ebpbackup")
            dl2.value.save_as(bpath)
            rdir = os.path.join(tmp, "restored-2fa")
            rest2 = subprocess.run([EBP_BIN, "backup", "restore", bpath, rdir], input=ADMIN_PW + "\n",
                                   capture_output=True, text=True, timeout=60)
            assert rest2.returncode == 0, (rest2.stdout, rest2.stderr)
            import sqlite3
            con = sqlite3.connect(os.path.join(rdir, "ebp.sqlite3"))
            for tbl in ("admin_totp", "admin_recovery_codes"):
                n = con.execute(f"SELECT count(*) FROM {tbl}").fetchone()[0]
                assert n == 0, f"{tbl} must not travel in a backup ({n} rows)"
            con.close()
            # Turn it off again (so the rest of the run signs in with the password only).
            page.click('#tabs button[data-v="set"]')
            page.locator("#twofa-card").get_by_role("button", name="Turn off", exact=True).click()
            page.wait_for_selector("#tf-off-pw")
            page.fill("#tf-off-pw", ADMIN_PW)
            page.fill("#tf-off-code", totp_code(secret, cur + 1))
            page.get_by_role("button", name="Turn off two-step sign-in").click()
            wait_for(lambda: "Off (password only)" in page.inner_text("#twofa-card"), "turned off")
            assert not page.is_visible("#bk-code"), "no code box once it is off"
            page.context.set_extra_http_headers({})

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

        @step("webhook retry: shop down when the payment lands, back later: delivered once, valid signature")
        def _():
            inv = new_invoice("hook-retry-1", 1800)
            HOOK["fail"] = True
            try:
                ctl(action="pay_ark", address=inv["ark_address"], sats=1800)
                wait_for(lambda: status_of(inv)["status"] == "paid", "invoice paid while the shop is down")
                wait_for(
                    lambda: any(json.loads(b).get("invoice_id") == inv["id"] for b in HOOK["failed_bodies"]),
                    "a first (refused) delivery attempt",
                )
                code, rows = http("GET", f"{BASE}/api/v1/webhook-deliveries?limit=50", headers=auth())
                mine = [d for d in rows if d["invoice_id"] == inv["id"] and d["event"] == "invoice.paid"]
                assert len(mine) == 1, mine
                assert mine[0]["status"] == "pending" and mine[0]["attempts"] >= 1, mine[0]
                assert not any(json.loads(r["body"]).get("invoice_id") == inv["id"] for r in received), "nothing delivered yet"
            finally:
                HOOK["fail"] = False
            # The first retry is scheduled ~30 s after the refused attempt.
            wait_for(
                lambda: any(
                    r["event"] == "invoice.paid" and json.loads(r["body"]).get("invoice_id") == inv["id"] for r in received
                ),
                "the retry to arrive once the shop is back",
                timeout=90,
            )
            time.sleep(3)
            got = [r for r in received if r["event"] == "invoice.paid" and json.loads(r["body"]).get("invoice_id") == inv["id"]]
            assert len(got) == 1, f"delivered {len(got)} times"
            assert verify_sig(state["hook_secret"], got[0]), "retried delivery must carry a valid signature"
            code, rows = http("GET", f"{BASE}/api/v1/webhook-deliveries?limit=50", headers=auth())
            mine = [d for d in rows if d["invoice_id"] == inv["id"] and d["event"] == "invoice.paid"]
            assert mine[0]["status"] == "delivered", mine[0]

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

        @step("hung barkd: health and invoice status still answer fast; the payment is credited once barkd wakes")
        def _():
            inv = new_invoice("hang-1", 1700)
            ctl(action="set", key="hang", value=True)
            try:
                ctl(action="pay_ark", address=inv["ark_address"], sats=1700)
                t0 = time.time()
                code, h = http("GET", f"{BASE}/api/v1/health", timeout=9)
                took = time.time() - t0
                assert code == 200 and took < 6, (code, took)
                assert h["wallet"]["reachable"] is False and h["status"] == "degraded", h
                t0 = time.time()
                assert status_of(inv)["status"] == "pending"
                assert time.time() - t0 < 3, "invoice status must not wait for the wallet"
            finally:
                ctl(action="set", key="hang", value=False)
            wait_for(lambda: status_of(inv)["status"] == "paid", "payment credited after barkd wakes", timeout=90)

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
            wait_for(lambda: page.is_visible("#login-form") or page.is_visible("#v-home"), "login or home")
            # The session is stored server-side, so it should survive a restart.
            assert page.is_visible("#v-home"), "signed-in session should survive a server restart"

        @step("crash: the server is killed (kill -9) at random moments around a payment; after a restart each payment is credited once")
        def _():
            import random
            import sqlite3
            rng = random.Random(20261108)
            _, before = http("GET", f"{BASE}/api/v1/unmatched-payments", headers=auth())
            invoices = []
            for i in range(8):
                sats = 1000 + i * 7
                inv = new_invoice(f"crash-{i}", sats)
                invoices.append((inv, sats))
                ctl(action="pay_ark", address=inv["ark_address"], sats=sats)
                time.sleep(rng.uniform(0, 1.2))  # the watcher looks every second
                state["ebp"].kill()  # no goodbye: SIGKILL
                state["ebp"].wait(timeout=10)
                state["ebp"] = start_ebp()
            for inv, sats in invoices:
                got = wait_for(lambda: status_of(inv)["status"] == "paid" and status_of(inv), f"{inv['id']} credited after the crashes", timeout=60)
                assert got["paid_sats"] == sats, ("credited once, for the exact amount", got)
            # Nobody was credited twice or left over as an unmatched payment.
            _, after = http("GET", f"{BASE}/api/v1/unmatched-payments", headers=auth())
            assert len(after) == len(before), ("no new unmatched payments", before, after)
            # One notification record per paid invoice (a delivery may be repeated
            # after a crash, but there is never more than one record).
            _, rows = http("GET", f"{BASE}/api/v1/webhook-deliveries?limit=200", headers=auth())
            for inv, _s in invoices:
                mine = [d for d in rows if d["invoice_id"] == inv["id"] and d["event"] == "invoice.paid"]
                assert len(mine) == 1, (inv["id"], mine)
            # The database is sound.
            con = sqlite3.connect(f"file:{os.path.join(tmp, 'data', 'ebp.sqlite3')}?mode=ro", uri=True)
            assert con.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            assert con.execute("SELECT count(*) FROM invoices WHERE status = 'paid' AND paid_sats < amount_sats").fetchone()[0] == 0
            con.close()

        @step("underpayment is not marked paid")
        def _():
            inv = new_invoice("under-1", 5000)
            ctl(action="pay_ark", address=inv["ark_address"], sats=2000)
            got = wait_for(lambda: status_of(inv)["paid_sats"] == 2000 and status_of(inv), "partial credited")
            assert got["status"] != "paid", got

        @step("admin: create a test payment from the Payments tab, pay it, see it paid in the list")
        def _():
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.click('#tabs button[data-v="orders"]')
            page.fill("#tp-sats", "1500")
            page.click("#tp-go")
            wait_for(lambda: page.is_visible("#tp-link"), "checkout link shown")
            href = page.get_attribute("#tp-link", "href")
            assert "/i/" in href, href
            inv_id = href.rstrip("/").split("/")[-1]
            code, inv = http("GET", f"{BASE}/i/{inv_id}/json")
            assert code == 200, (code, inv)
            ctl(action="pay_ark", address=inv["ark_address"], sats=1500)
            wait_for(lambda: status_of({"id": inv_id})["status"] == "paid", "test payment paid")
            page.click('#tabs button[data-v="home"]')
            page.click('#tabs button[data-v="orders"]')
            wait_for(lambda: "paid" in page.inner_text("#orders"), "paid shown in list")

        @step("backup: encrypted download from the settings tab opens with the admin password only")
        def _():
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.click('#tabs button[data-v="set"]')
            page.wait_for_selector("#bk-go")
            bk_pw = ADMIN_PW  # one password: the file is locked with the admin password
            page.fill("#bk-admin", ADMIN_PW)
            with page.expect_download(timeout=60000) as dl:
                page.click("#bk-go")
            path = os.path.join(tmp, "backup.ebpbackup")
            dl.value.save_as(path)
            wait_for(lambda: "Saved as" in page.inner_text("#bk-msg"), "saved message")
            assert "NOT inside" in page.inner_text("#bk-msg"), "unmanaged mode must say wallet files are not included"
            assert not page.input_value("#bk-admin"), "password fields should be cleared"
            raw = open(path, "rb").read()
            assert b"SQLite format 3" not in raw and WORDS[0].encode() not in raw
            good = subprocess.run([EBP_BIN, "backup", "verify", path], input=bk_pw + "\n",
                                  capture_output=True, text=True, timeout=60)
            assert good.returncode == 0 and "ebp.sqlite3" in good.stdout, (good.stdout, good.stderr)
            bad = subprocess.run([EBP_BIN, "backup", "verify", path], input="not the password\n",
                                 capture_output=True, text=True, timeout=60)
            assert bad.returncode != 0, "a wrong password must not open the backup"
            out = os.path.join(tmp, "restored")
            rest = subprocess.run([EBP_BIN, "backup", "restore", path, out], input=bk_pw + "\n",
                                  capture_output=True, text=True, timeout=60)
            assert rest.returncode == 0, (rest.stdout, rest.stderr)
            assert os.path.getsize(os.path.join(out, "ebp.sqlite3")) > 0

        @step("password: a mistyped current password is a message (still signed in); a real change signs out and the new one works; changed back")
        def _():
            newpw = "a second long password for testing"
            page.context.set_extra_http_headers({"x-forwarded-for": "10.77.0.2"})
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.click('#tabs button[data-v="set"]')
            page.wait_for_selector("#pw-save")
            page.fill("#pw-cur", "definitely not the password")
            page.fill("#pw-new", newpw)
            page.click("#pw-save")
            wait_for(lambda: page.is_visible("#pw-msg") and page.inner_text("#pw-msg").strip(), "message for a wrong current password")
            assert not page.is_visible("#login-form"), "a mistyped password must not look like being signed out"
            page.fill("#pw-cur", ADMIN_PW)
            page.fill("#pw-new", newpw)
            page.click("#pw-save")
            page.wait_for_selector("#login-form:not(.hidden)")
            page.fill("#pw", ADMIN_PW)
            page.click("#login-btn")
            wait_for(lambda: page.inner_text("#login-err").strip() != "", "old password refused")
            page.fill("#pw", newpw)
            page.click("#login-btn")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.click('#tabs button[data-v="set"]')
            page.wait_for_selector("#pw-save")
            page.fill("#pw-cur", newpw)
            page.fill("#pw-new", ADMIN_PW)
            page.click("#pw-save")
            page.wait_for_selector("#login-form:not(.hidden)")
            page.fill("#pw", ADMIN_PW)
            page.click("#login-btn")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.context.set_extra_http_headers({})

        @step("payout: the wizard set the first address; a change in Settings waits and can be cancelled")
        def _():
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.click('#tabs button[data-v="set"]')
            page.wait_for_selector("#po-go")
            first = "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"
            second = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
            wait_for(lambda: first in page.inner_text("#po-now"), "address from the wizard shown")
            page.fill("#po-addr", second)
            page.fill("#po-pw", ADMIN_PW)
            page.click("#po-go")
            wait_for(lambda: page.is_visible("#po-cancel"), "pending change shown with a cancel button")
            assert first in page.inner_text("#po-now"), "old address stays in use while a change waits"
            page.click("#po-cancel")
            wait_for(lambda: not page.is_visible("#po-cancel"), "cancelled")
            page.click('#tabs button[data-v="home"]')
            wait_for(lambda: "Payout address is set" in page.inner_text("#health"), "home says the payout address is set")

        @step("pause + key limit: pausing refuses new payments, the shop key can be limited to an address, both undone")
        def _():
            page.goto(f"{BASE}/admin")
            page.wait_for_selector("#v-home:not(.hidden)")
            page.click('#tabs button[data-v="set"]')
            page.get_by_role("button", name="Pause new payments").click()
            wait_for(lambda: "Paused" in page.inner_text("#pause-card"), "paused")
            page.click('#tabs button[data-v="home"]')
            wait_for(lambda: page.is_visible("#paused-card"), "paused notice on Home")
            code, body = http("POST", f"{BASE}/api/v1/invoices", {"order_id": "paused-1", "sats": 1000}, auth())
            assert code == 409 and "paused" in json.dumps(body), (code, body)
            page.locator("#paused-card").get_by_role("button", name="Resume payments").click()
            wait_for(lambda: not page.is_visible("#paused-card"), "resumed")
            new_invoice("paused-2", 1000)
            # The shop key limited to another address stops working from here.
            page.click('#tabs button[data-v="hooks"]')
            page.click("#keylock-box summary")
            page.fill("#kl-addr", "198.51.100.7")
            page.click("#kl-save")
            wait_for(lambda: "Saved" in page.inner_text("#kl-msg"), "limit saved")
            code, _ = http("GET", f"{BASE}/api/v1/invoices?limit=1", headers=auth())
            assert code == 401, code
            page.click('#tabs button[data-v="home"]')
            page.click('#tabs button[data-v="hooks"]')
            wait_for(lambda: "was refused" in page.inner_text("#kl-blocked"), "the refused address is shown to the owner")
            page.fill("#kl-addr", "")
            page.click("#kl-save")
            wait_for(lambda: "No limit" in page.inner_text("#kl-msg"), "limit removed")
            code, _ = http("GET", f"{BASE}/api/v1/invoices?limit=1", headers=auth())
            assert code == 200, code

        if DEVICE:

            @step("phone: every admin tab and the checkout page fit the screen")
            def _():
                problems = []
                inv = new_invoice("phone-1", 2100)
                page.goto(f"{BASE}/admin")
                if page.is_visible("#login-form"):
                    page.fill("#pw", ADMIN_PW)
                    page.click("#login-btn")
                page.wait_for_selector("#v-home:not(.hidden)")
                for tab in ("home", "orders", "alerts", "hooks", "set"):
                    page.click(f'#tabs button[data-v="{tab}"]')
                    time.sleep(0.8)
                    problems.append(phone_audit(f"admin tab {tab}"))
                page.goto(f"{BASE}/i/{inv['id']}")
                time.sleep(1.0)
                problems.append(phone_audit("checkout pending"))
                ctl(action="pay_ark", address=inv["ark_address"], sats=2100)
                wait_for(lambda: status_of(inv)["status"] == "paid", "paid")
                time.sleep(3)
                problems.append(phone_audit("checkout paid"))
                problems = [p for p in problems if p]
                assert not problems, problems

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
