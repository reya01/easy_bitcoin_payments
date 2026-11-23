#!/usr/bin/env python3
"""The Home tab's payment cheer (the little weightlifter), with the server mocked.

It must: stay quiet on the first look (nothing already paid counts), appear for a payment that
arrives while Home is open, fire at most once a minute, never show for old payments, and fail
quietly (list error, missing picture) without a script error or any change to the page.
Run: python3 tests/e2e/payment_cheer.py [screenshot-dir]
"""
import json, os, sys, datetime
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
PNG = open(os.path.join(ROOT, "assets", "mascot", "lifter.png"), "rb").read()
ORIGIN = "http://ebp.test"
SHOTS = sys.argv[1] if len(sys.argv) > 1 else ""


def iso(seconds_ago=0):
    return (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=seconds_ago)).strftime("%Y-%m-%dT%H:%M:%SZ")


def inv(n, status="paid", ago=5, sats=21000):
    return {"id": f"inv_{n}", "status": status, "amount_sats": sats, "paid_at": iso(ago) if status in ("paid", "overpaid") else None,
            "created_at": iso(ago + 60), "fiat_amount": None}


def run(pw, name, steps, picture=True):
    b = pw.chromium.launch()
    p = b.new_context(viewport={"width": 390, "height": 1200}).new_page()
    errs = []
    state = {"list": [inv(1, ago=5)], "fail": False}
    p.on("pageerror", lambda e: errs.append(str(e)))
    watch_csp(p, errs)
    p.clock.install()

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        if path in ("/admin", "/admin/"):
            return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
        if serve_asset(route, path):
            return
        if path == "/assets/mascot/lifter.png":
            if picture:
                return route.fulfill(status=200, content_type="image/png", body=PNG)
            return route.fulfill(status=404, body="")
        if path == "/admin/session":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"authenticated": True, "csrf": "tok", "password_set": True, "twofa": False}))
        if path == "/api/v1/admin/status":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"checks": [], "counters": {}, "wallet": {}, "mode": "live", "usd_per_btc": 67000}))
        if path == "/api/v1/invoices":
            if state["fail"]:
                return route.fulfill(status=500, content_type="application/json", body='{"error":"boom"}')
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(state["list"]))
        return route.fulfill(status=200, content_type="application/json", body="{}")

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/admin")
    p.wait_for_timeout(500)
    problems = []

    def check():
        p.clock.run_for(30000)  # the page checks every 30 seconds
        p.wait_for_timeout(400)

    def shown():
        return p.is_visible("#cheer")

    try:
        steps(p, problems, state, check, shown)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if SHOTS and not problems:
        os.makedirs(SHOTS, exist_ok=True)
    if errs:
        problems.append(f"script errors: {errs}")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def main():
    ok = True
    with sync_playwright() as pw:
        def s_main(p, pr, st, check, shown):
            check()  # a payment already there when Home opened (recent, even) is not news
            if shown(): pr.append("an already-paid payment must not cheer on the first look")
            st["list"] = [inv(2, ago=3), inv(1, ago=4000)]
            check()
            if not shown(): pr.append("a new payment must show the cheer")
            else:
                t = p.inner_text("#cheer")
                if "New payment!" not in t: pr.append(f"unexpected text: {t!r}")
                if "21,000" not in t: pr.append(f"the amount is missing: {t!r}")
                if not p.evaluate("(function(){var i=document.querySelector('#cheer img');return !!i && i.complete && i.naturalWidth>0 && new URL(i.src).pathname==='/assets/mascot/lifter.png'})()"):
                    pr.append("the lifter picture did not load from the built-in path")
                if SHOTS: p.locator("#cheer").screenshot(path=os.path.join(SHOTS, "cheer.png"))
            st["list"] = [inv(3, ago=2), inv(2, ago=3), inv(1, ago=4000)]
            check()
            if shown(): pr.append("a second payment inside a minute must not cheer again")
            st["list"] = [inv(4, ago=1), inv(3, ago=2), inv(2, ago=3), inv(1, ago=4000)]
            check()
            if not shown(): pr.append("a minute after the first, the next payment cheers again")
        ok &= run(pw, "cheers for a new payment, quiet on first look, once a minute at most", s_main)

        def s_old(p, pr, st, check, shown):
            check()
            st["list"] = [inv(2, ago=3600), inv(1, ago=4000)]; check()
            if shown(): pr.append("a payment paid long ago must not cheer")
            st["list"] = [inv(5, status="pending"), inv(2, ago=3600), inv(1, ago=4000)]; check()
            st["list"] = [inv(5, status="underpaid"), inv(2, ago=3600), inv(1, ago=4000)]; check()
            st["list"] = [inv(5, status="expired"), inv(2, ago=3600), inv(1, ago=4000)]; check()
            if shown(): pr.append("only fully paid payments cheer")
        ok &= run(pw, "old, pending, underpaid and expired payments do not cheer", s_old)

        def s_fail(p, pr, st, check, shown):
            check()
            st["fail"] = True
            st["list"] = [inv(2, ago=2)]
            check()
            if shown(): pr.append("a failed list must not show anything")
            if not p.is_visible("#health"): pr.append("Home must stay usable")
            st["fail"] = False
            check()
            if not shown(): pr.append("recovers on the next look")
        ok &= run(pw, "a failing list is ignored; the next look still works", s_fail)

        def s_nopic(p, pr, st, check, shown):
            check()
            st["list"] = [inv(2, ago=2), inv(1, ago=4000)]; check()
            p.wait_for_timeout(300)
            if shown(): pr.append("a missing picture must hide the cheer, not show a broken box")
        ok &= run(pw, "a missing picture hides the cheer quietly", s_nopic, picture=False)

        def s_away(p, pr, st, check, shown):
            check()
            p.evaluate("document.querySelector('#tabs button[data-v=orders]').click()")
            p.wait_for_timeout(300)
            st["list"] = [inv(2, ago=2), inv(1, ago=4000)]
            check()
            if p.evaluate("!document.getElementById('v-home').classList.contains('hidden')"): pr.append("test setup: should be off Home")
            if shown(): pr.append("no cheer while another tab is open")
        ok &= run(pw, "no cheer on other tabs", s_away)
    sys.exit(0 if ok else 1)


main()
