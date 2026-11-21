#!/usr/bin/env python3
"""The Home tab's Update card and Version row, with the server mocked.

Checks the update helper flow: two decisions (Start update, then Update now),
the Now/New comparison and signature line, links to inspect further on GitHub,
password plus code for the second decision, a clear reason when refused, the
install line when the helper is missing, and the reload after Done.
Run: python3 tests/e2e/update_card.py [screenshot-dir]
"""
import json, os, sys, time
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
ORIGIN = "http://ebp.test"
SHOTS = sys.argv[1] if len(sys.argv) > 1 else ""
CUR, NEW = "0.2.1-rc2", "0.2.2-rc1"
DIGEST = "sha256:" + "c" * 64
COMMIT = "d" * 40
RUN = "https://github.com/reya01/easy_bitcoin_payments/actions/runs/123/attempts/1"
LATEST = {"version": NEW, "url": "https://github.com/reya01/easy_bitcoin_payments/releases/tag/v" + NEW, "security": False, "prerelease": True}


def version(helper=1, status=None, current=CUR, available=True):
    return {"current": current, "check_enabled": True, "latest": LATEST if available else None,
            "update_available": available, "open_orders": 0, "host": "docker",
            "helper_version": helper, "helper_status": status,
            "repo_url": "https://github.com/reya01/easy_bitcoin_payments"}


def ready(req="ab" * 8):
    return {"state": "ready", "code": "ok", "request_id": req, "running": CUR, "version": NEW,
            "digest": DIGEST, "commit": COMMIT, "run_url": RUN, "at": 1}


def run_case(pw, name, handler_for, checks, twofa=False, shot=""):
    b = pw.chromium.launch()
    p = b.new_context(viewport={"width": 390, "height": 1400}).new_page()
    errs, calls = [], []
    p.on("pageerror", lambda e: errs.append(str(e)))
    watch_csp(p, errs)

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        if path in ("/admin", "/admin/"):
            return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
        if serve_asset(route, path):
            return
        calls.append((req.method, path, req.post_data))
        if path == "/admin/session":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"authenticated": True, "csrf": "tok", "password_set": True, "twofa": twofa}))
        if path == "/api/v1/admin/status":
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"checks": [], "counters": {}, "wallet": {}, "mode": "live", "usd_per_btc": 67000}))
        out = handler_for(path, req, calls)
        if out is None:
            return route.fulfill(status=200, content_type="application/json", body="{}")
        return route.fulfill(status=out[0], content_type="application/json", body=json.dumps(out[1]))

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/admin")
    p.wait_for_timeout(600)
    problems = []
    try:
        checks(p, problems, calls)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if SHOTS and shot:
        p.locator("#update").screenshot(path=os.path.join(SHOTS, shot + ".png"))
    if errs:
        problems.append(f"script errors: {errs}")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def want(pr, what, got, text):
    if text.lower() not in (got or "").lower():
        pr.append(f"{what}: wanted '{text}', saw '{(got or '')[:160]}'")


def body_of(calls, path):
    sent = [c for c in calls if c[0] == "POST" and c[1] == path]
    return json.loads(sent[-1][2]) if sent else None


def main():
    ok = True
    with sync_playwright() as pw:
        # 1. Helper missing: the install line, and today's two commands as the fallback.
        def missing(path, req, calls):
            return (200, version(helper=None)) if path == "/api/v1/admin/version" else None

        def c_missing(p, pr, calls):
            t = p.inner_text("#update")
            want(pr, "move to the folder first", t, "cd /opt/ebp")
            want(pr, "install line", t, "./scripts/install-update-helper.sh")
            want(pr, "fallback", t, "./scripts/update.sh --check " + NEW)
            if p.locator('#update button:text-is("Start update")').count(): pr.append("no Start update without the helper")
            want(pr, "version row", p.inner_text("#version-row"), "Update available")
        ok &= run_case(pw, "helper missing: install line and Console fallback", missing, c_missing, shot="update-missing")

        # 2. Up to date: the Version row says so and the card stays hidden.
        def uptodate(path, req, calls):
            return (200, version(available=False)) if path == "/api/v1/admin/version" else None

        def c_uptodate(p, pr, calls):
            want(pr, "version row", p.inner_text("#version-row"), CUR + " · up to date")
            if p.is_visible("#update"): pr.append("card should be hidden when up to date")
        ok &= run_case(pw, "up to date: Version row says so, no card", uptodate, c_uptodate)

        # 3. First decision, then the comparison, then the second decision with password and code.
        state = {"checked": False}

        def flow(path, req, calls):
            if path == "/admin/update/check":
                state["checked"] = True
                return 200, {"request_id": "ab" * 8}
            if path == "/admin/update/apply":
                return 200, {"request_id": "cd" * 8}
            if path == "/api/v1/admin/version":
                return 200, version(status=ready() if state["checked"] else None)
            return None

        def c_flow(p, pr, calls):
            want(pr, "offer", p.inner_text("#update"), "Update available: version " + NEW)
            if body_of(calls, "/admin/update/check"): pr.append("nothing may be checked before Start update")
            p.click('#update button:text-is("Start update")')
            p.wait_for_timeout(300)
            if (body_of(calls, "/admin/update/check") or {}).get("version") != NEW: pr.append("Start update must ask to check the new version")
            want(pr, "checking", p.inner_text("#update"), "Checking version " + NEW)
            p.wait_for_timeout(3600)
            t = p.inner_text("#update")
            for words in ["Now", CUR, "New", NEW, "Signature OK", "release workflow for v" + NEW, COMMIT[:12], DIGEST, "Inspect it further on GitHub"]:
                want(pr, "comparison", t, words)
            hrefs = p.eval_on_selector_all("#update a", "as => as.map(a => a.href)")
            if LATEST["url"] not in hrefs: pr.append("release page link missing")
            if RUN not in hrefs: pr.append("workflow run link missing")
            if body_of(calls, "/admin/update/apply"): pr.append("nothing may be applied before Update now")
            p.fill("#upd-pw", "my admin password")
            p.fill("#upd-code", "123456")
            p.click('#update button:text-is("Update now")')
            p.wait_for_timeout(300)
            b = body_of(calls, "/admin/update/apply") or {}
            if b.get("digest") != DIGEST or b.get("version") != NEW: pr.append(f"apply must send the shown digest and version, sent {b}")
            if b.get("password") != "my admin password" or b.get("code") != "123456": pr.append("apply must send the password and the code")
            want(pr, "updating", p.inner_text("#update"), "Updating to version " + NEW)
        ok &= run_case(pw, "two decisions: check, compare, then password and code", flow, c_flow, twofa=True, shot="update-confirm-step")

        # 3b. The confirm screen as it looks (for the screenshot), without two-step sign-in: no code box.
        def at_ready(path, req, calls):
            return (200, version(status=ready())) if path == "/api/v1/admin/version" else None

        def c_ready(p, pr, calls):
            if p.is_visible("#upd-code"): pr.append("no code box without two-step sign-in")
            if not p.is_visible("#upd-pw"): pr.append("the password is always asked")
        ok &= run_case(pw, "confirm screen: password always, code only with two-step sign-in", at_ready, c_ready, shot="update-confirm")

        # 4. A wrong password is shown and the button works again.
        def wrong_pw(path, req, calls):
            if path == "/admin/update/apply":
                return 400, {"error": {"message": "That is not your admin password."}}
            return (200, version(status=ready())) if path == "/api/v1/admin/version" else None

        def c_wrong(p, pr, calls):
            p.fill("#upd-pw", "nope"); p.click('#update button:text-is("Update now")'); p.wait_for_timeout(300)
            want(pr, "message", p.inner_text("#update"), "not your admin password")
            if p.is_disabled('#update button:text-is("Update now")'): pr.append("button must work again after a refusal")
        ok &= run_case(pw, "wrong password: clear message, try again", wrong_pw, c_wrong)

        # 5. Refused checks give one clear reason each.
        for code, words in [("older", "older than the one you run"), ("signature_failed", "signature check failed"),
                            ("digest_changed", "changed since you checked"), ("no_digest", "Could not download")]:
            st = {"state": "refused", "code": code, "request_id": "ab" * 8, "running": CUR, "version": NEW,
                  "digest": "", "commit": "", "run_url": "", "at": 1}

            def refused(path, req, calls, st=st):
                return (200, version(status=st)) if path == "/api/v1/admin/version" else None

            def c_ref(p, pr, calls, words=words):
                t = p.inner_text("#update")
                want(pr, "reason", t, words)
                want(pr, "nothing changed", t, "Nothing was changed")
                if p.is_visible("#upd-pw"): pr.append("no password box after a refusal")
            ok &= run_case(pw, f"refused ({code}): one clear reason", refused, c_ref, shot="update-refused" if code == "signature_failed" else "")

        # 6. Done: after the restart the new version answers, the card says so and the page reloads.
        stage = {"n": 0}

        def done(path, req, calls):
            if path == "/admin/update/apply":
                return 200, {"request_id": "cd" * 8}
            if path == "/api/v1/admin/version":
                stage["n"] += 1
                if any(c[1] == "/admin/update/apply" for c in calls) and stage["n"] > 3:
                    return 200, version(current=NEW, available=False, status=dict(ready(), state="done", running=NEW, version=NEW, at=int(time.time())))
                return 200, version(status=ready())
            return None

        def c_done(p, pr, calls):
            p.fill("#upd-pw", "pw"); p.click('#update button:text-is("Update now")')
            p.wait_for_timeout(14000)
            want(pr, "done", p.inner_text("body"), "Updated to version " + NEW)
            loads = len([c for c in calls if c[1] == "/admin/session"])
            if loads < 2: pr.append("the page should reload itself after Done")
        ok &= run_case(pw, "done: says so, then reloads the page", done, c_done)
    sys.exit(0 if ok else 1)


main()
