#!/usr/bin/env python3
"""The Invoices tab (in testing), with the server mocked, at phone width.

It must: stay hidden until turned on in Settings; make an invoice with the next
number filled in; show the payment request text (amount, business, note, number,
Lightning with its exact end time, Ark) WITHOUT the customer name; draw the
picture (only the facts, QR codes that decode when OpenCV is available); always
show the send-privately warning under an open invoice; drop the Lightning part
once it has run out; offer no sharing for a paid invoice; copy text and picture
to the clipboard; and never scroll sideways or trip the Content-Security-Policy.
Run: python3 tests/e2e/invoices_tab.py [screenshot-dir]
"""
import base64, datetime, json, os, sys
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
# localhost is a secure context, so the clipboard works in the test browser.
ORIGIN = "http://localhost:8099"
SHOTS = sys.argv[1] if len(sys.argv) > 1 else ""
BOLT11 = "lntbs10u1p5jfqgqdqydxyzxqr23sqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqpmn3vw"
ARK = "tark1qqexampleexampleexampleexampleexampleexampleexample7f3k"


def iso(seconds_from_now):
    t = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=seconds_from_now)
    return t.strftime("%Y-%m-%dT%H:%M:%SZ")


def inv(n, status="pending", ln_left=3 * 3600, name="Sam Rivera", refresh=False):
    return {
        "id": f"inv_{n}", "invoice_number": str(n), "customer_name": name, "note": "Bathroom tap repair",
        "status": status, "amount_sats": 25000, "paid_sats": 25000 if status == "paid" else 0,
        "fiat_currency": None, "fiat_amount": None, "ark_address": ARK,
        "lightning_invoice": BOLT11, "lightning_expires_at": iso(ln_left), "can_refresh_lightning": refresh,
        "lightning_fee_estimate_sat": 20, "expires_at": iso(24 * 3600), "created_at": iso(-60),
        "paid_at": iso(-10) if status == "paid" else None, "store_name": "Sam's Plumbing",
        "ark_server": None, "test_mode": True,
    }


def run(pw, name, steps):
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 390, "height": 1400}, timezone_id="America/New_York")
    ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=ORIGIN)
    p = ctx.new_page()
    errs, posts = [], []
    state = {"enabled": False, "list": [], "next": "3", "made": None}
    p.on("pageerror", lambda e: errs.append(str(e)))
    watch_csp(p, errs)

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        if path in ("/admin", "/admin/"):
            return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
        if serve_asset(route, path):
            return
        if path == "/admin/session":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"authenticated": True, "csrf": "tok", "password_set": True, "twofa": False}))
        if path == "/api/v1/admin/invoices/settings":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps({
                "enabled": state["enabled"], "next_number": state["next"], "customer_names": ["Sam Rivera", "Alex"],
                "store_name": "Sam's Plumbing", "open_hours": 24,
                "default_currency": "USD", "currencies": ["USD", "EUR"], "ark_server": None, "test_mode": True}))
        if path == "/api/v1/admin/invoices/enabled":
            body = json.loads(req.post_data or "{}")
            posts.append(("enabled", body, req.headers.get("x-ebp-csrf")))
            state["enabled"] = bool(body.get("enabled"))
            return route.fulfill(status=200, content_type="application/json", body=json.dumps({"enabled": state["enabled"]}))
        if path == "/api/v1/admin/invoices" and req.method == "POST":
            body = json.loads(req.post_data or "{}")
            posts.append(("create", body, req.headers.get("x-ebp-csrf")))
            made = state["made"] or inv(int(body.get("invoice_number") or 3), name=body.get("customer_name") or None)
            state["list"] = [made] + state["list"]
            state["next"] = "4"
            return route.fulfill(status=201, content_type="application/json", body=json.dumps(made))
        if path == "/api/v1/admin/invoices":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(state["list"]))
        if path == "/api/v1/admin/status":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"checks": [], "counters": {}, "wallet": {}, "mode": "test", "usd_per_btc": 67000}))
        if path.startswith("/api/v1/invoices") or path.startswith("/api/v1/admin/settlements") or path.startswith("/api/v1/audit-log"):
            return route.fulfill(status=200, content_type="application/json", body="[]")
        return route.fulfill(status=200, content_type="application/json", body="{}")

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/admin")
    p.wait_for_timeout(600)
    problems = []
    try:
        steps(p, problems, state, posts)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script or policy errors: {errs}")
    if p.evaluate("document.documentElement.scrollWidth - window.innerWidth") > 0:
        problems.append("the page scrolls sideways")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def open_tab(p):
    p.click("#tab-inv")
    p.wait_for_timeout(400)


def decode_picture(p):
    """Decode the QR codes in the drawn picture with OpenCV, if it is installed."""
    try:
        import cv2, numpy as np  # noqa: E401
    except Exception:  # noqa: BLE001
        return None
    data = p.evaluate("document.getElementById('inv-pic').src")
    raw = base64.b64decode(data.split(",", 1)[1])
    img = cv2.imdecode(np.frombuffer(raw, np.uint8), cv2.IMREAD_COLOR)
    ok, texts, _, _ = cv2.QRCodeDetector().detectAndDecodeMulti(img)
    return sorted(t for t in texts if t) if ok else []


def main():
    ok = True
    with sync_playwright() as pw:
        def s_turn_on(p, pr, st, posts):
            if p.is_visible("#tab-inv"): pr.append("the tab must be hidden while invoices are off")
            p.click('#tabs button[data-v="set"]'); p.wait_for_timeout(300)
            p.click("#fold-inv summary"); p.wait_for_timeout(200)
            p.click("#inv-set button"); p.wait_for_timeout(400)
            if not posts or posts[-1][0] != "enabled" or posts[-1][1] != {"enabled": True}: pr.append(f"turning on must post enabled=true: {posts}")
            elif posts[-1][2] != "tok": pr.append("the change must carry the CSRF token")
            if not p.is_visible("#tab-inv"): pr.append("the tab must appear once turned on")
        ok &= run(pw, "hidden until turned on in Settings", s_turn_on)

        def s_make(p, pr, st, posts):
            st["enabled"] = True
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            if p.input_value("#inv-num") != "3": pr.append(f"the next number must be filled in: {p.input_value('#inv-num')!r}")
            opts = p.eval_on_selector_all("#inv-unit option", "os => os.map(o => o.value)")
            if opts != ["sats", "USD", "EUR"]: pr.append(f"the amount menu must be sats first, then the priced currencies: {opts}")
            if p.input_value("#inv-unit") != "sats": pr.append("the amount menu starts on sats")
            lim = p.inner_text("#inv-limits")
            for must in ["24 hours", "Lightning or Ark", "exact end time", "make a fresh one", "still reaches your wallet"]:
                if must not in lim: pr.append(f"the time limits note is missing {must!r}")
            if p.query_selector("#inv-days"): pr.append("the 'stays open for' menu must be gone")
            p.fill("#inv-amt", "25000"); p.fill("#inv-note", "Bathroom tap repair")
            p.click("#inv-more summary"); p.wait_for_timeout(150); p.fill("#inv-cust", "Sam Rivera")
            p.click("#inv-go"); p.wait_for_timeout(600)
            sent = [x for x in posts if x[0] == "create"]
            if not sent: pr.append("nothing was sent"); return
            body = sent[-1][1]
            want = {"sats": 25000, "note": "Bathroom tap repair", "invoice_number": "3", "customer_name": "Sam Rivera"}
            for k, v in want.items():
                if body.get(k) != v: pr.append(f"{k}: sent {body.get(k)!r}, want {v!r}")
            text = p.inner_text("#inv-text")
            for must in ["25,000 sats", "Sam's Plumbing", "Bathroom tap repair", "(invoice 3)", "Pay with Lightning", "It works until", "UTC", BOLT11, "Pay with Ark", ARK, "TEST INVOICE"]:
                if must not in text: pr.append(f"the text is missing {must!r}")
            if "Sam Rivera" in text: pr.append("the customer name must not be in the text sent to the customer")
            if "AMOUNT: 25,000 sats\n" + ARK not in text: pr.append("the Ark amount must be on its own line right above the address")
            big = p.evaluate("(function(){var e=document.querySelector('#inv-ark-amt strong');return e?[e.textContent,parseFloat(getComputedStyle(e).fontSize),getComputedStyle(e).fontWeight]:null})()")
            if not big or big[0] != "25,000 sats" or big[1] < 24 or int(big[2]) < 700: pr.append(f"the Ark amount must be big and bold on screen: {big}")
            w = p.inner_text("#inv-warn") if p.is_visible("#inv-warn") else ""
            for must in ["Send this privately", "Proton Mail", "Signal", "cannot stop"]:
                if must not in w: pr.append(f"the warning is missing {must!r}")
            if not p.evaluate("(function(){var i=document.getElementById('inv-pic');return !!i&&i.complete&&i.naturalWidth>=1000})()"):
                pr.append("the picture was not drawn")
            texts = decode_picture(p)
            if texts is not None and texts and texts != sorted([ARK, "lightning:" + BOLT11]):
                pr.append(f"the picture's QR codes decode to {texts}")
            # Copy text, then copy picture: both land on the clipboard.
            p.click("text=Copy text"); p.wait_for_timeout(300)
            if p.evaluate("navigator.clipboard.readText()") != text: pr.append("Copy text must copy exactly the shown text")
            p.click("text=Copy picture"); p.wait_for_timeout(800)
            types = p.evaluate("navigator.clipboard.read().then(function(items){return items.map(function(i){return i.types.join(',')})})")
            if not any("image/png" in t for t in types): pr.append(f"Copy picture must put a PNG on the clipboard: {types}")
            if p.input_value("#inv-num") != "4": pr.append("after making one, the next number must be filled in")
            if "3 · Sam Rivera" not in p.inner_text("#inv-list"): pr.append("the new invoice must be listed with its customer")
            if p.evaluate("getComputedStyle(document.querySelector('#inv-list .pl')).flexDirection") != "column": pr.append("the amount and the invoice line must be on two lines")
            if SHOTS:
                os.makedirs(SHOTS, exist_ok=True)
                p.locator("#inv-out").screenshot(path=os.path.join(SHOTS, "invoice-made.png"))
                data = p.evaluate("document.getElementById('inv-pic').src")
                open(os.path.join(SHOTS, "invoice-picture.png"), "wb").write(base64.b64decode(data.split(",", 1)[1]))
                p.locator("#inv-new").screenshot(path=os.path.join(SHOTS, "invoice-form.png"))
        ok &= run(pw, "makes an invoice: text without the customer name, picture, warning, copy", s_make)

        def s_expired(p, pr, st, posts):
            st["enabled"] = True
            st["list"] = [inv(5, ln_left=-3600, refresh=True)]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            text = p.inner_text("#inv-text")
            if "Lightning" in text or BOLT11 in text: pr.append("a run-out Lightning invoice must not be in the text")
            if ARK not in text: pr.append("the Ark address must stay")
            if not p.is_visible("text=Make a fresh Lightning invoice"): pr.append("the fresh-invoice button must show")
            if not p.is_visible("#inv-warn"): pr.append("the warning must show under every open invoice")
            texts = decode_picture(p)
            if texts is not None and texts and texts != [ARK]: pr.append(f"the picture must hold only the Ark code: {texts}")
        ok &= run(pw, "a run-out Lightning invoice leaves Ark only and offers a fresh one", s_expired)

        def s_paid(p, pr, st, posts):
            st["enabled"] = True
            st["list"] = [inv(6, status="paid")]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            if p.is_visible("#inv-text") or p.is_visible("text=Copy text") or p.is_visible("#inv-pic"):
                pr.append("a paid invoice must offer nothing to send")
            if "Paid" not in p.inner_text("#inv-out"): pr.append("a paid invoice must say Paid")
        ok &= run(pw, "a paid invoice offers nothing to send", s_paid)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
