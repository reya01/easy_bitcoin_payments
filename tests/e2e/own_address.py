#!/usr/bin/env python3
"""Settings, Use your own address (in testing), with the server mocked, at phone width.

It must: stay off until turned on; suggest pay.<shop's domain>; show the one record
to add (A, the name's first part, the server's IP, DNS only) with Copy buttons and
Cloudflare steps; change nothing until a check passes; show the server's plain
reason when a check fails; then offer to serve the name next to the current
address; offer to make it the main address with the passkey warning and the
authenticator code; while watching, show the days left, warn when the shop still
uses the old address, and offer Go back (asked first) and Keep the backup a little
longer; and never scroll sideways or trip the Content-Security-Policy.
Run: python3 tests/e2e/own_address.py [screenshot-dir]
"""
import json, os, sys
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
ORIGIN = "http://localhost:8099"
SHOTS = sys.argv[1] if len(sys.argv) > 1 else ""
SSLIP = "203-0-113-7.sslip.io"


def base_state():
    return {
        "enabled": False, "helper_installed": True, "main": SSLIP, "backups": [], "server_ip": "203.0.113.7",
        "suggestion": "pay.myshop.com", "step": "idle", "name": None,
        "record": {"type": "A", "name": None, "value": "203.0.113.7", "proxy": "DNS only"},
        "switched_at": None, "days_left": None, "failures": 0, "last_problem": None, "last_ok_at": None,
        "shop_host": None, "shop_moved": None, "keep_backup_until": None,
    }


def run(pw, name, steps, start=None):
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 390, "height": 1600})
    ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=ORIGIN)
    p = ctx.new_page()
    errs, posts = [], []
    st = base_state()
    st.update(start or {})
    box = {"st": st, "check": {"finding": {"result": "ready"}, "words": "Ready: this name reaches your server."}}
    p.on("pageerror", lambda e: errs.append(str(e)))
    watch_csp(p, errs)

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        j = lambda body, code=200: route.fulfill(status=code, content_type="application/json", body=json.dumps(body))
        if path in ("/admin", "/admin/"):
            return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
        if serve_asset(route, path):
            return
        if path == "/admin/session":
            return j({"authenticated": True, "csrf": "tok", "password_set": True, "twofa": False})
        if path == "/api/v1/admin/address":
            return j(box["st"])
        if path.startswith("/api/v1/admin/address/"):
            body = json.loads(req.post_data or "{}")
            what = path.rsplit("/", 1)[1]
            posts.append((what, body, req.headers.get("x-ebp-csrf")))
            s = box["st"]
            if what == "enabled":
                s["enabled"] = body["enabled"]
                return j({"enabled": s["enabled"]})
            if what == "check":
                s["name"] = body["name"].strip().lower()
                if box["check"]["finding"]["result"] == "ready":
                    s["step"] = "checked"
                return j(box["check"])
            if what == "serve":
                s.update(step="serving", backups=[s["name"]])
                return j({"requested": True})
            if what == "make-main":
                s.update(step="main", main=s["name"], backups=[SSLIP], switched_at=1, days_left={"watch": 14, "backup": 30})
                return j({"requested": True})
            if what == "go-back":
                s.update(step="serving", main=SSLIP, backups=[s["name"]])
                return j({"requested": True})
            if what == "keep-backup":
                s["days_left"]["backup"] = 10
                return j({"keep_backup_until": 99})
            if what == "cancel":
                box["st"] = base_state() | {"enabled": True}
                return j({"cancelled": True})
        if path == "/api/v1/admin/status":
            return j({"checks": [], "counters": {}, "wallet": {}, "mode": "test", "usd_per_btc": 67000})
        if path.startswith("/api/v1/invoices") or path.startswith("/api/v1/admin/settlements") or path.startswith("/api/v1/audit-log"):
            return route.fulfill(status=200, content_type="application/json", body="[]")
        return j({})

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/admin")
    p.wait_for_timeout(600)
    p.click('#tabs button[data-v="set"]')
    p.wait_for_timeout(400)
    p.evaluate("document.getElementById('fold-od').open = true")
    p.wait_for_timeout(200)
    problems = []
    try:
        steps(p, problems, box, posts)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script or policy errors: {errs}")
    if p.evaluate("document.documentElement.scrollWidth - window.innerWidth") > 0:
        problems.append("the page scrolls sideways")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def text(p):
    return p.inner_text("#od-set")


def main():
    ok = True
    with sync_playwright() as pw:
        def s_off(p, pr, box, posts):
            t = text(p)
            if "Turn this on" not in t or "get your shop running first" not in t:
                pr.append(f"off: explains and offers to turn on: {t[:200]!r}")
            if p.locator("#od-name").count():
                pr.append("nothing to type until it is turned on")
            p.click("#od-set >> text=Turn this on"); p.wait_for_timeout(400)
            if posts[-1][:2] != ("enabled", {"enabled": True}) or posts[-1][2] != "tok":
                pr.append(f"turning on sends the switch with the CSRF token: {posts[-1:]}")
            if p.input_value("#od-name") != "pay.myshop.com":
                pr.append("suggests pay.<shop's domain>")
        ok &= run(pw, "off until turned on, then suggests pay.<shop domain>", s_off)

        def s_record_and_check(p, pr, box, posts):
            t = text(p)
            for want in ("Type", "A", "Name", "pay", "IPv4 address", "203.0.113.7", "Proxy status", "DNS only",
                         "Main address: " + SSLIP):
                if want not in t:
                    pr.append(f"the record shows {want!r}")
            p.fill("#od-name", "shop.example.com"); p.wait_for_timeout(100)
            if "shop" not in p.inner_text(".odrec"):
                pr.append("the record's name follows what is typed")
            p.click("#od-set summary:has-text('Cloudflare')"); p.wait_for_timeout(100)
            if "orange cloud OFF" not in text(p):
                pr.append("Cloudflare steps say to turn the proxy off")
            p.locator(".odrow", has_text="203.0.113.7").locator("button").click(); p.wait_for_timeout(200)
            if p.evaluate("navigator.clipboard.readText()") != "203.0.113.7":
                pr.append("Copy copies the IP")
            # A failing check: its reason is shown, and nothing more is offered.
            box["check"] = {"finding": {"result": "cloudflare_proxy"}, "words": "This name goes through Cloudflare's proxy. In Cloudflare, open the record and turn the orange cloud off (Proxy status: DNS only), then check again."}
            p.click("#od-set button:text-is('Check')"); p.wait_for_timeout(400)
            if "turn the orange cloud off" not in p.inner_text("#od-msg"):
                pr.append("a failed check shows the server's reason")
            if p.locator("#od-set button", has_text="Serve").count():
                pr.append("nothing more is offered until the check passes")
            if any(x[0] in ("serve", "make-main") for x in posts):
                pr.append("a failed check changes nothing")
            # Passes.
            box["check"] = {"finding": {"result": "ready"}, "words": "Ready: this name reaches your server."}
            p.click("#od-set button:text-is('Check')"); p.wait_for_timeout(400)
            if posts[-1][:2] != ("check", {"name": "shop.example.com"}):
                pr.append(f"check sends the typed name: {posts[-1:]}")
            if "Ready" not in p.inner_text("#od-msg"):
                pr.append("a passed check says so")
            serve = p.locator("#od-set button", has_text="Serve shop.example.com next to my current address")
            if not serve.count():
                pr.append("after a passed check, serving it is offered"); return
            if SHOTS: p.locator("#fold-od").screenshot(path=os.path.join(SHOTS, "own-address-check.png"))
            serve.click(); p.wait_for_timeout(500)
            if "Both addresses work now" not in text(p):
                pr.append("then both addresses work")
        ok &= run(pw, "shows the record, explains a failed check, serves only after a pass", s_record_and_check,
                  {"enabled": True})

        def s_make_main(p, pr, box, posts):
            t = text(p)
            for want in ("Passkeys you made on the old address only work there", "Links you already sent keep working"):
                if want not in t:
                    pr.append(f"before switching it says {want!r}")
            p.fill("#od-code", "123456")
            p.click("#od-set >> text=Make pay.myshop.com my main address"); p.wait_for_timeout(500)
            if posts[-1][:2] != ("make-main", {"code": "123456"}):
                pr.append(f"make main sends the code: {posts[-1:]}")
            t = text(p)
            if "Watching it for 14 more days" not in t or "given up in 30 days" not in t:
                pr.append(f"after the switch: watch and backup days: {t[:300]!r}")
            if SHOTS: p.locator("#fold-od").screenshot(path=os.path.join(SHOTS, "own-address-main.png"))
        ok &= run(pw, "makes it the main address with the passkey warning and the code", s_make_main,
                  {"enabled": True, "step": "serving", "name": "pay.myshop.com", "backups": ["pay.myshop.com"]})

        def s_watch(p, pr, box, posts):
            t = text(p)
            if "Your shop still uses the old address" not in t or "https://pay.myshop.com" not in t:
                pr.append("warns while the shop still calls the old address")
            p.click("#od-set >> text=Keep the backup a little longer"); p.wait_for_timeout(400)
            if posts[-1][0] != "keep-backup":
                pr.append("keep longer is sent")
            p.click("#od-set >> text=Go back to the backup address"); p.wait_for_timeout(300)
            if not p.is_visible(".dlg"):
                pr.append("going back asks first"); return
            p.click("text=Keep my own address"); p.wait_for_timeout(300)
            if any(x[0] == "go-back" for x in posts):
                pr.append("declining changes nothing")
            p.click("#od-set >> text=Go back to the backup address"); p.wait_for_timeout(300)
            p.click(".dlg button:text-is('Go back')"); p.wait_for_timeout(500)
            if posts[-1][0] != "go-back":
                pr.append("go back is sent once confirmed")
        ok &= run(pw, "while watching: shop warning, keep longer, go back (asked first)", s_watch,
                  {"enabled": True, "step": "main", "name": "pay.myshop.com", "main": "pay.myshop.com",
                   "backups": [SSLIP], "switched_at": 1, "days_left": {"watch": 0, "backup": 2},
                   "shop_host": SSLIP, "shop_moved": False})

        def s_problem(p, pr, box, posts):
            t = text(p)
            if "had a problem" not in t or "is kept" not in t or "not given up while there are problems" not in t:
                pr.append(f"a failed check keeps the backup and says so: {t[:300]!r}")
        ok &= run(pw, "a failed watch check keeps the backup", s_problem,
                  {"enabled": True, "step": "main", "name": "pay.myshop.com", "main": "pay.myshop.com",
                   "backups": [SSLIP], "switched_at": 1, "days_left": {"watch": 5, "backup": 20}, "failures": 1,
                   "last_problem": "https://pay.myshop.com did not answer"})

        def s_no_helper(p, pr, box, posts):
            if "install-update-helper.sh" not in text(p):
                pr.append("explains how to install the helper")
        ok &= run(pw, "without the helper it says what to run", s_no_helper, {"enabled": True, "helper_installed": False})
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
