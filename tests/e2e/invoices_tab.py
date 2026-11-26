#!/usr/bin/env python3
"""The Invoices tab (in testing), with the server mocked, at phone width.

It must: stay hidden until turned on in Settings; make an invoice with the next
number filled in; show the payment request text (amount, business, note, number,
Lightning with its exact end time, Ark) WITHOUT the customer name; draw the
picture (only the facts, QR codes that decode when OpenCV is available); always
show the send-privately warning under an open invoice; drop the Lightning part
once it has run out; offer no sharing for a paid invoice; copy text and picture
to the clipboard; keep an open invoice up to date while it is on screen (and stop
asking once the tab changes); ask "is this right?" before an unusually large amount;
fill the form for the rest of a part payment and for "Invoice again" without making
anything; mark late money handled from the invoice; offer a receipt once paid; and
never scroll sideways or trip the Content-Security-Policy.
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


def run(pw, name, steps, clock=False):
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 390, "height": 1400}, timezone_id="America/New_York")
    ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=ORIGIN)
    p = ctx.new_page()
    if clock:
        p.clock.install()
    errs, posts = [], []
    state = {"enabled": False, "list": [], "next": "3", "made": None, "settings": {}, "one": {}, "gets": []}
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
                "default_currency": "USD", "currencies": ["USD", "EUR"], "ark_server": None, "test_mode": True,
                **state["settings"]}))
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
        if path.startswith("/api/v1/unmatched-payments/") and path.endswith("/resolve"):
            posts.append(("resolve", path.split("/")[4], req.headers.get("x-ebp-csrf")))
            return route.fulfill(status=200, content_type="application/json", body=json.dumps({"resolved": True}))
        if path.startswith("/api/v1/admin/invoices/") and req.method == "GET":
            iid = path.rsplit("/", 1)[1]
            state["gets"].append(iid)
            found = state["one"].get(iid) or next((x for x in state["list"] if x["id"] == iid), None)
            if found is None:
                return route.fulfill(status=404, content_type="application/json", body="{}")
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(found))
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

        def s_late(p, pr, st, posts):
            st["enabled"] = True
            late = inv(7, status="expired")
            late["created_at"], late["expires_at"] = iso(-30 * 3600), iso(-6 * 3600)
            late["payments"] = [{"rail": "ark", "amount_sats": 25000, "detected_at": iso(-60), "counted": False,
                                 "reason": "paid_after_expiry", "handled": False}]
            late["late_sats"], late["late_at"] = 25000, iso(-60)
            part = inv(8, status="underpaid")
            part["paid_sats"] = 10000
            part["payments"] = [{"rail": "lightning", "amount_sats": 10000, "detected_at": iso(-30), "counted": True,
                                 "reason": None, "handled": False}]
            st["list"] = [late, part]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            rows = p.locator("#inv-list .inv-row")
            first = rows.nth(0).inner_text()
            if "Expired" not in first or "Paid late" not in first:
                pr.append(f"an invoice paid after its time shows both Expired and Paid late: {first!r}")
            if "25,000 sats arrived by Ark after its 24 hours ran out" not in first:
                pr.append(f"the row says what arrived: {first!r}")
            if "Part paid: 10,000 of 25,000 sats" not in rows.nth(1).inner_text():
                pr.append("a part-paid invoice says how much of it was paid")
            rows.nth(0).click(); p.wait_for_timeout(300)
            if not p.is_visible("#inv-late") or "listed under Alerts" not in p.inner_text("#inv-late"):
                pr.append("the opened invoice explains the late money")
            story = p.inner_text("#inv-story")
            for want in ("Made", "Time to pay ran out", "arrived after it stopped taking payments"):
                if want not in story: pr.append(f"the story must say {want!r}: {story!r}")
            if story.index("Time to pay ran out") > story.index("arrived after"):
                pr.append("the story is in time order")
            if p.is_visible("#inv-text"): pr.append("an expired invoice offers nothing to send")
            rows.nth(1).click(); p.wait_for_timeout(300)
            if "Paid 10,000 sats by Lightning" not in p.inner_text("#inv-story"):
                pr.append("a counted payment is in the story")
            if not p.is_visible("#inv-text"): pr.append("a part-paid invoice can still be sent again")
            if SHOTS:
                rows.nth(0).click(); p.wait_for_timeout(200)
                p.locator("#inv-list").screenshot(path=os.path.join(SHOTS, "invoice-late-list.png"))
                p.locator("#inv-out").screenshot(path=os.path.join(SHOTS, "invoice-late-story.png"))
        ok &= run(pw, "money after the invoice closed shows on it: Expired and Paid late, and its story", s_late)

        def s_live(p, pr, st, posts):
            st["enabled"] = True
            st["list"] = [inv(9)]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            left = p.inner_text("#inv-left") if p.is_visible("#inv-left") else ""
            if "left to pay" not in left or not left.startswith(("23 h", "24 h")):
                pr.append(f"an open invoice says how long is left: {left!r}")
            # Nothing changed: the screen stays as it is.
            p.clock.fast_forward(16000); p.wait_for_timeout(300)
            if "inv_9" not in st["gets"]: pr.append("an open invoice on screen is asked for again")
            # The customer pays: it turns Paid by itself.
            paid = inv(9, status="paid")
            st["one"]["inv_9"] = paid; st["list"] = [paid]
            p.clock.fast_forward(16000); p.wait_for_timeout(400)
            out = p.inner_text("#inv-out")
            if "Paid" not in p.inner_text("#inv-out .pills"): pr.append(f"it must turn Paid without a reload: {out[:200]!r}")
            if "Paid. The money is in your wallet." not in out: pr.append("it must say it was paid")
            if p.is_visible("#inv-left"): pr.append("a paid invoice shows no time left")
            # Closed now, and on another tab: no more asking.
            n = len(st["gets"])
            p.clock.fast_forward(60000); p.wait_for_timeout(300)
            if len(st["gets"]) != n: pr.append("a paid invoice is not asked for again")
            st["list"] = [inv(11)]
            p.click('#tabs button[data-v="home"]'); open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            p.click('#tabs button[data-v="home"]'); p.wait_for_timeout(200)
            n = len(st["gets"])
            p.clock.fast_forward(60000); p.wait_for_timeout(300)
            if len(st["gets"]) != n: pr.append("leaving the tab stops the asking")
        ok &= run(pw, "an open invoice on screen keeps itself up to date and shows the time left", s_live, clock=True)

        def s_big(p, pr, st, posts):
            st["enabled"] = True
            st["settings"] = {"largest_sats": 25000, "usd_per_btc": 67000, "prices": {"USD": 67000, "EUR": 62000}}
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            # A usual amount: no question.
            p.fill("#inv-amt", "30000"); p.click("#inv-go"); p.wait_for_timeout(400)
            if p.is_visible(".dlg"): pr.append("a usual amount must not ask")
            if len([x for x in posts if x[0] == "create"]) != 1: pr.append("a usual amount is made straight away")
            # A typo with an extra zero or two: asked first, and Go back makes nothing.
            p.fill("#inv-amt", "2500000"); p.click("#inv-go"); p.wait_for_timeout(300)
            if not p.is_visible(".dlg"): pr.append("a very large amount must ask first"); return
            d = p.inner_text(".dlg")
            for want in ("2,500,000 sats", "about $1,675", "more than 10 times your largest invoice so far (25,000 sats)"):
                if want not in d: pr.append(f"the question must say {want!r}: {d!r}")
            if SHOTS: p.screenshot(path=os.path.join(SHOTS, "invoice-big-amount.png"))
            p.click("text=Go back and check"); p.wait_for_timeout(300)
            if len([x for x in posts if x[0] == "create"]) != 1: pr.append("Go back must make nothing")
            # In dollars: the dollar amount itself is checked.
            p.select_option("#inv-unit", "USD"); p.fill("#inv-amt", "2400"); p.click("#inv-go"); p.wait_for_timeout(300)
            d = p.inner_text(".dlg") if p.is_visible(".dlg") else ""
            if "USD 2400" not in d or "about $2,400" not in d: pr.append(f"a large dollar amount asks too: {d!r}")
            p.click("text=Yes, make it"); p.wait_for_timeout(400)
            made = [x for x in posts if x[0] == "create"]
            if len(made) != 2 or made[-1][1].get("fiat") != {"currency": "USD", "amount": "2400"}:
                pr.append(f"Yes must make it as typed: {made[-1:]}")
        ok &= run(pw, "asks before an unusually large amount", s_big)

        def s_rest(p, pr, st, posts):
            st["enabled"] = True
            part = inv(10, status="expired")
            part["paid_sats"] = 10000
            part["created_at"], part["expires_at"] = iso(-30 * 3600), iso(-6 * 3600)
            part["payments"] = [{"rail": "lightning", "amount_sats": 10000, "detected_at": iso(-29 * 3600), "counted": True,
                                 "reason": None, "handled": False, "unmatched_id": None}]
            st["list"] = [part]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            btn = p.locator("text=Make a new invoice for the 15,000 sats still owed")
            if not btn.is_visible(): pr.append("an expired part-paid invoice offers to bill the rest"); return
            btn.click(); p.wait_for_timeout(300)
            got = {k: p.input_value(k) for k in ("#inv-amt", "#inv-unit", "#inv-note", "#inv-cust", "#inv-num")}
            want = {"#inv-amt": "15000", "#inv-unit": "sats", "#inv-note": "Rest of invoice 10: Bathroom tap repair",
                    "#inv-cust": "Sam Rivera", "#inv-num": "3"}
            if got != want: pr.append(f"the form must be filled for the rest: {got}")
            if "still owed" not in p.inner_text("#inv-msg"): pr.append("the form says where it came from")
            if any(x[0] == "create" for x in posts): pr.append("filling the form must not make an invoice")
            # The rest arrived too late: offering to bill it again could charge twice.
            part["late_sats"], part["late_at"] = 15000, iso(-60)
            part["payments"].append({"rail": "ark", "amount_sats": 15000, "detected_at": iso(-60), "counted": False,
                                     "reason": "paid_after_expiry", "handled": False, "unmatched_id": "unm_r"})
            st["list"] = [part]
            p.click('#tabs button[data-v="home"]'); open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            if p.locator("#inv-out button", has_text="still owed").count(): pr.append("no 'bill the rest' once late money has arrived")
        ok &= run(pw, "bills the rest of a part payment by filling the form", s_rest)

        def s_again(p, pr, st, posts):
            st["enabled"] = True
            old = inv(6, status="paid")
            old["fiat_currency"], old["fiat_amount"] = "USD", "25.00"
            st["list"] = [old]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            p.click("text=Invoice again"); p.wait_for_timeout(300)
            got = {k: p.input_value(k) for k in ("#inv-amt", "#inv-unit", "#inv-note", "#inv-cust")}
            want = {"#inv-amt": "25.00", "#inv-unit": "USD", "#inv-note": "Bathroom tap repair", "#inv-cust": "Sam Rivera"}
            if got != want: pr.append(f"Invoice again copies the amount, note and customer: {got}")
            if "today's price" not in p.inner_text("#inv-msg"): pr.append("a money amount says it is priced again today")
            if any(x[0] == "create" for x in posts): pr.append("Invoice again must not make anything by itself")
        ok &= run(pw, "Invoice again fills the form from an old invoice", s_again)

        def s_handled(p, pr, st, posts):
            st["enabled"] = True
            late = inv(7, status="expired")
            late["created_at"], late["expires_at"] = iso(-30 * 3600), iso(-6 * 3600)
            pay = {"rail": "ark", "amount_sats": 25000, "detected_at": iso(-60), "counted": False,
                   "reason": "paid_after_expiry", "handled": False, "unmatched_id": "unm_1"}
            late["payments"], late["late_sats"], late["late_at"] = [pay], 25000, iso(-60)
            st["list"] = [late]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            done = json.loads(json.dumps(late)); done["payments"][0]["handled"] = True
            st["one"]["inv_7"] = done
            p.click("#inv-out >> text=Mark handled"); p.wait_for_timeout(500)
            res = [x for x in posts if x[0] == "resolve"]
            if res != [("resolve", "unm_1", "tok")]: pr.append(f"Mark handled marks its Alerts row, with the CSRF token: {res}")
            txt = p.inner_text("#inv-late")
            if "You marked it handled." not in txt: pr.append(f"the invoice says it was handled: {txt!r}")
            if p.locator("#inv-out >> text=Mark handled").count(): pr.append("no button once handled")
            if "Paid late" not in p.inner_text("#inv-out .pills"): pr.append("it still shows Paid late")
            if "marked handled under Alerts" not in p.inner_text("#inv-story"): pr.append("the story keeps the payment, as handled")
        ok &= run(pw, "late money can be marked handled from the invoice", s_handled)

        def s_receipt(p, pr, st, posts):
            st["enabled"] = True
            st["list"] = [inv(6, status="paid")]
            p.reload(); p.wait_for_timeout(600)
            open_tab(p)
            p.click("#inv-list .inv-row"); p.wait_for_timeout(300)
            r = p.inner_text("#inv-receipt") if p.is_visible("#inv-receipt") else ""
            for want in ("TEST RECEIPT", "Receipt from Sam's Plumbing", "Invoice 6: Bathroom tap repair", "Paid in full: 25,000 sats", "Paid on", "Thank you!"):
                if want not in r: pr.append(f"the receipt must say {want!r}: {r!r}")
            for never in ("Sam Rivera", ARK, BOLT11):
                if never in r: pr.append(f"the receipt must not hold {never!r}")
            p.click("text=Copy receipt"); p.wait_for_timeout(300)
            if p.evaluate("navigator.clipboard.readText()") != r: pr.append("Copy receipt copies the shown receipt")
            if "Receipt copied." not in p.inner_text("#inv-out-msg"): pr.append("it says the receipt was copied")
            if SHOTS: p.locator("#inv-out").screenshot(path=os.path.join(SHOTS, "invoice-receipt.png"))
        ok &= run(pw, "a paid invoice offers a receipt without the customer name or addresses", s_receipt)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
