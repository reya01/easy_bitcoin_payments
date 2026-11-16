#!/usr/bin/env python3
"""Admin page behaviours, with the server mocked (no ebp binary needed).

Checks: the in-page confirmation on destructive buttons (and that nothing happens
until it is confirmed), the "Password changed" message, the "Forgot your password?"
link, retry buttons when a card cannot load, real links to the guides, and that
problem messages are announced.  Run: python3 tests/e2e/dashboard_states.py
"""
import json, os, sys
from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
QR = open(os.path.join(ROOT, "assets", "qr.js")).read()
ORIGIN = "http://ebp.test"


def run_case(pw, name, handler_for, checks):
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 390, "height": 844})
    p = ctx.new_page()
    errs, calls = [], []
    p.on("pageerror", lambda e: errs.append(str(e)))
    p.add_init_script("window.confirm = function () { window.__nativeConfirm = true; return false; };")

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        if path in ("/admin", "/admin/"):
            return route.fulfill(status=200, content_type="text/html", body=HTML)
        if path == "/assets/qr.js":
            return route.fulfill(status=200, content_type="application/javascript", body=QR)
        calls.append((req.method, path, req.post_data))
        out = handler_for(path, req, calls)
        if out is None:
            return route.fulfill(status=200, content_type="application/json", body="{}")
        code, body = out
        return route.fulfill(status=code, content_type="application/json", body=json.dumps(body))

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/admin")
    p.wait_for_timeout(500)
    problems = []
    try:
        checks(p, problems, calls)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script errors: {errs}")
    if p.evaluate("window.__nativeConfirm||false"):
        problems.append("the browser's own confirm() box was used")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def want(pr, what, got, text):
    if text.lower() not in (got or "").lower():
        pr.append(f"{what}: wanted '{text}', saw '{(got or '')[:100]}'")


def signed_in(path, req, calls, **over):
    if path == "/admin/session":
        return 200, {"authenticated": True, "csrf": "tok", "password_set": True}
    if path == "/api/v1/admin/status":
        return 200, {"checks": [], "counters": {}, "wallet": {}, "mode": "test", "usd_per_btc": 67000}
    return None


def count(calls, method, path):
    return len([c for c in calls if c[0] == method and c[1] == path])


def main():
    ok = True
    with sync_playwright() as pw:
        def hooks_handler(path, req, calls):
            if path == "/api/v1/stores/webhook":
                return 200, {"webhook_secret": "whsec_NEW"}
            if path == "/api/v1/stores/rotate-key":
                return 200, {"api_key": "ebp_NEWKEY"}
            return signed_in(path, req, calls)

        def c_confirms(p, pr, calls):
            p.click('#tabs button[data-v="hooks"]')
            p.fill("#hook-url", "https://shop.example/wc-api/ebp")
            # signing secret
            p.click("#hook-rotate")
            want(pr, "title", p.inner_text("#dlg-title"), "Make a new signing secret?")
            want(pr, "names the other website", p.inner_text("#dlg-body"), "WooCommerce site")
            if p.evaluate("document.activeElement.textContent") != "Keep current": pr.append("focus should start on the safe button")
            p.click("#dlg-back button:text-is(\"Keep current\")")
            if p.is_visible("#dlg-back"): pr.append("dialog should close")
            p.wait_for_timeout(100)
            if count(calls, "PUT", "/api/v1/stores/webhook"): pr.append("nothing may be sent before confirming")
            p.click("#hook-rotate"); p.keyboard.press("Escape")
            if p.is_visible("#dlg-back"): pr.append("Escape should cancel")
            p.click("#hook-rotate"); p.click("#dlg-back button:text-is(\"Make new secret\")"); p.wait_for_timeout(200)
            sent = [c for c in calls if c[1] == "/api/v1/stores/webhook" and c[0] == "PUT"]
            if len(sent) != 1 or "rotate_secret" not in (sent[0][2] or ""): pr.append(f"confirmed rotate should send once, saw {sent}")
            # turn off
            p.click("#hook-off")
            want(pr, "off title", p.inner_text("#dlg-title"), "Turn off payment notifications?")
            want(pr, "off says orders stay unpaid", p.inner_text("#dlg-body"), "paid orders there will stay unpaid")
            p.click("#dlg-back button:text-is(\"Keep on\")"); p.wait_for_timeout(100)
            if count(calls, "PUT", "/api/v1/stores/webhook") != 1: pr.append("Keep on must not turn anything off")
            p.click("#hook-off"); p.click("#dlg-back .btns button:last-child"); p.wait_for_timeout(200)
            if count(calls, "PUT", "/api/v1/stores/webhook") != 2: pr.append("confirmed turn-off should send")
            # API key
            p.click("#rotate")
            want(pr, "key title", p.inner_text("#dlg-title"), "Make a new API key?")
            p.click("#dlg-back button:text-is(\"Keep current\")"); p.wait_for_timeout(100)
            if count(calls, "POST", "/api/v1/stores/rotate-key"): pr.append("key must not rotate before confirming")
            p.click("#rotate"); p.click("#dlg-back button:text-is(\"Make new key\")"); p.wait_for_timeout(200)
            if count(calls, "POST", "/api/v1/stores/rotate-key") != 1: pr.append("confirmed key rotation should send once")
            want(pr, "new key shown", p.inner_text("#newkey"), "ebp_NEWKEY")
            # emergency stop and move-now (hidden panels: click through script)
            p.evaluate("document.getElementById('ex-stop').click()")
            want(pr, "stop title", p.inner_text("#dlg-title"), "Stop the emergency withdrawal?")
            p.click("#dlg-back button:text-is(\"Keep going\")"); p.wait_for_timeout(100)
            if count(calls, "POST", "/admin/exit/cancel"): pr.append("withdrawal must not stop before confirming")
            p.evaluate("document.getElementById('ex-stop').click()"); p.click("#dlg-back .btns button:last-child"); p.wait_for_timeout(200)
            if count(calls, "POST", "/admin/exit/cancel") != 1: pr.append("confirmed stop should send")
            p.evaluate("document.getElementById('sweep-go').click()")
            want(pr, "move title", p.inner_text("#dlg-title"), "Move your money now?")
            p.click("#dlg-back button:text-is(\"Not now\")"); p.wait_for_timeout(100)
            if count(calls, "POST", "/admin/sweep"): pr.append("nothing may move before confirming")
            p.evaluate("document.getElementById('sweep-go').click()"); p.click("#dlg-back button:text-is(\"Move it\")"); p.wait_for_timeout(200)
            if count(calls, "POST", "/admin/sweep") != 1: pr.append("confirmed move should send")

        ok &= run_case(pw, "destructive buttons ask in the page first, and nothing happens until confirmed", hooks_handler, c_confirms)

        def pw_handler(path, req, calls):
            if path == "/admin/password":
                return 200, {}
            if path == "/admin/logout":
                return 200, {}
            return signed_in(path, req, calls)

        def c_pw(p, pr, calls):
            p.click('#tabs button[data-v="set"]')
            p.fill("#pw-cur", "old"); p.fill("#pw-new", "new password that is long enough")
            p.click("#pw-save"); p.wait_for_timeout(300)
            if not p.is_visible("#v-login"): pr.append("should go back to sign-in")
            want(pr, "says it worked", p.inner_text("#login-note"), "Password changed. Sign in with your new password.")

        def c_logout(p, pr, calls):
            p.click("#logout"); p.wait_for_timeout(300)
            if p.is_visible("#login-note"): pr.append("signing out must not claim the password changed")

        ok &= run_case(pw, "password changed: the sign-in screen says so", pw_handler, c_pw)
        ok &= run_case(pw, "plain sign-out shows no password message", pw_handler, c_logout)

        def login_handler(path, req, calls):
            if path == "/admin/session":
                return 200, {"authenticated": False, "password_set": True}
            return None

        def c_forgot(p, pr, calls):
            t = p.inner_text("#v-login")
            want(pr, "forgot heading", t, "Forgot your password?")
            want(pr, "names the place", t, "payment server's Console")
            href = p.get_attribute("#forgot-link", "href") or ""
            if "IF_SOMETHING_LOOKS_WRONG.md#2-" not in href: pr.append(f"link should reach the reset guide section, saw {href}")
            if p.get_attribute("#forgot-link", "target") != "_blank" or "noopener" not in (p.get_attribute("#forgot-link", "rel") or ""): pr.append("link should open safely in a new tab")
            p.fill("#pw", "wrong"); p.click("#login-btn"); p.wait_for_timeout(300)
            if p.get_attribute("#login-err", "role") != "alert": pr.append("login error should be announced")

        ok &= run_case(pw, "sign-in screen: Forgot your password? with a real link, errors announced", login_handler, c_forgot)

        # cards that cannot load say so and retry
        state = {"n": 0}

        def retry_handler(path, req, calls):
            if path in ("/api/v1/admin/version", "/admin/reconcile/status", "/admin/alerts/status"):
                if calls.count(calls[-1]) == 1 and not state.get(path):
                    state[path] = True
                    return 500, {"error": {"message": "nope"}}
                if path == "/api/v1/admin/version":
                    return 200, {"current": "0.2.0-rc2", "update_available": False}
                if path == "/admin/reconcile/status":
                    return 200, {"paid_orders": 1, "swept_sats": 500, "due": True, "days": 7, "paid_sats": 600, "last_checked_at": None}
                return 200, {"state": "undecided"}
            return signed_in(path, req, calls)

        def c_retry(p, pr, calls):
            for sel, text in (("#update", "We couldn't check for updates."), ("#recon", "We couldn't check your backup."), ("#phone-home", "We couldn't check your alert settings.")):
                want(pr, sel + " message", p.inner_text(sel), text)
                if not p.is_visible(sel + " button"): pr.append(sel + " needs a Try again button")
            p.click("#update button"); p.click("#recon button"); p.click("#phone-home button"); p.wait_for_timeout(400)
            if "couldn't" in p.inner_text("#update").lower(): pr.append("updates card should recover after Try again")
            want(pr, "backup card recovers", p.inner_text("#recon"), "weekly wallet check")

        ok &= run_case(pw, "cards that fail to load say so and offer Try again", retry_handler, c_retry)

        def err_handler(path, req, calls):
            if path == "/api/v1/stores/webhook":
                return 500, {}
            return signed_in(path, req, calls)

        def c_docs(p, pr, calls):
            p.click('#tabs button[data-v="hooks"]')
            p.fill("#hook-url", "https://shop.example/x"); p.click("#hook-save"); p.wait_for_timeout(300)
            a = p.locator("#hook-msg a")
            if a.count() != 1: pr.append("the server-problem message should contain a link to Known issues")
            else:
                if "KNOWN_ISSUES.md" not in (a.get_attribute("href") or ""): pr.append("wrong link target")
            if p.get_attribute("#hook-msg", "role") != "alert": pr.append("problem messages should be announced")

        ok &= run_case(pw, "server-problem message links to Known issues and is announced", err_handler, c_docs)

        # ---- Emergency withdrawal: fee money and why Start is grey ----
        TB = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        ARK = "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"

        def exit_handler(mode="test", confirmed=0, pending=0, needed=894):
            def h(path, req, calls):
                if path == "/api/v1/admin/status":
                    return 200, {"checks": [], "counters": {}, "wallet": {}, "mode": mode, "usd_per_btc": 67000}
                if path == "/admin/exit/status":
                    return 200, {"min_exit_sats": 100000, "ark_sats": 500000, "fee": {"broadcast_sat": 894, "claim_sat": 28, "total_sat": 922, "txs_to_broadcast": 3},
                                 "onchain_confirmed_sats": confirmed, "onchain_pending_sats": pending, "deposit_needed_sats": needed, "deadline": 1900000000,
                                 "asp_connected": True, "two_step_on": True, "allowed": True, "refusal": None, "in_progress_since": None, "live": [],
                                 "claimable": False, "destination": None}
                if path == "/admin/exit/deposit-address":
                    return 200, {"address": TB}
                return signed_in(path, req, calls)
            return h

        def open_exit(p):
            p.click('#tabs button[data-v="set"]')
            p.evaluate("document.getElementById('ex-box').open = true")
            p.wait_for_timeout(700)

        def c_short(p, pr, calls):
            open_exit(p)
            want(pr, "needs line", p.inner_text("#ex-fee-rows"), "Fee money the server needs")
            want(pr, "needed amount", p.inner_text("#ex-fee-rows"), "894")
            want(pr, "what is there now", p.inner_text("#ex-fee-rows"), "On the server now (confirmed)")
            want(pr, "still to send", p.inner_text("#ex-fee-rows"), "Still to send")
            want(pr, "verdict says not enough", p.inner_text("#ex-fee-verdict"), "Not enough yet: send at least")
            want(pr, "address shows without a click", p.inner_text("#ex-addr"), TB)
            if not p.is_visible("#ex-qr img"): pr.append("the QR code for the deposit address should show")
            want(pr, "says it is not an Ark address", p.inner_text("#ex-addr-box"), "not an Ark address")
            if count(calls, "POST", "/admin/exit/deposit-address") != 1: pr.append("the address should be fetched once, not on every refresh")
            want(pr, "grey explained up front", p.inner_text("#ex-checks"), "grey until these are done (5 to go)")
            want(pr, "fee money is the first reason", p.inner_text("#ex-checks"), "send at least 894 sats")
            p.fill("#ex-dest", ARK)
            want(pr, "Ark address caught as you type", p.inner_text("#ex-dest-note"), "That is an Ark address")
            want(pr, "and listed as a reason", p.inner_text("#ex-checks"), "That is an Ark address")
            p.fill("#ex-dest", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")
            want(pr, "real address on play money", p.inner_text("#ex-dest-note"), "real-Bitcoin address, but this server uses play money")
            p.fill("#ex-dest", "not an address")
            want(pr, "nonsense caught", p.inner_text("#ex-dest-note"), "does not look like a Bitcoin address")
            p.fill("#ex-dest", TB)
            want(pr, "good address accepted", p.inner_text("#ex-dest-note"), "Looks like a regular Bitcoin address")
            p.fill("#ex-pw", "correct horse battery staple"); p.fill("#ex-code", "123456"); p.check("#ex-ok")
            if not p.is_disabled("#ex-start"): pr.append("Start must stay grey while the fee money is missing")
            want(pr, "one reason left, and it is the fee money", p.inner_text("#ex-checks"), "(1 to go)")
            want(pr, "reason names the amount", p.inner_text("#ex-checks"), "\u2717 Fee money: send at least 894 sats")

        ok &= run_case(pw, "emergency withdrawal: fee money amounts, deposit address, and why Start is grey", exit_handler(), c_short)

        def c_enough(p, pr, calls):
            open_exit(p)
            want(pr, "verdict says enough", p.inner_text("#ex-fee-verdict"), "Enough")
            if p.is_visible("#ex-addr-box"): pr.append("no deposit address is needed once the fee money is there")
            p.fill("#ex-dest", TB); p.fill("#ex-pw", "correct horse battery staple"); p.fill("#ex-code", "123456")
            if not p.is_disabled("#ex-start"): pr.append("Start must wait for the tick-box")
            want(pr, "tick-box is the one reason", p.inner_text("#ex-checks"), "Tick the box above the button")
            p.check("#ex-ok")
            if p.is_disabled("#ex-start"): pr.append("Start should be enabled when everything is done")
            want(pr, "ready message", p.inner_text("#ex-checks"), "Everything is ready")

        ok &= run_case(pw, "emergency withdrawal: enough fee money, Start turns on only when everything is done", exit_handler(confirmed=1200, needed=0), c_enough)

        def c_wait(p, pr, calls):
            open_exit(p)
            want(pr, "waiting row", p.inner_text("#ex-fee-rows"), "Sent, waiting for 1 confirmation")
            want(pr, "verdict says waiting, not missing", p.inner_text("#ex-fee-verdict"), "waiting for 1 confirmation")
            want(pr, "checklist says waiting", p.inner_text("#ex-checks"), "waiting for 1 confirmation")

        ok &= run_case(pw, "emergency withdrawal: money sent but not confirmed yet says so", exit_handler(confirmed=0, pending=900, needed=894), c_wait)

        def c_live(p, pr, calls):
            open_exit(p)
            p.fill("#ex-dest", TB)
            want(pr, "test address on real money", p.inner_text("#ex-dest-note"), "test-network address, but this server uses real Bitcoin")
            want(pr, "tells them what to use", p.inner_text("#ex-dest-note"), "bc1")
            p.fill("#ex-dest", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")
            want(pr, "real address accepted on real money", p.inner_text("#ex-dest-note"), "Looks like a regular Bitcoin address")

        ok &= run_case(pw, "emergency withdrawal: on a real-money page a test address is caught", exit_handler(mode="live"), c_live)

        def unpriceable_handler():
            base = exit_handler()
            def h(path, req, calls):
                if path == "/admin/exit/status":
                    return 200, {"min_exit_sats": 100000, "ark_sats": 500000, "fee": None, "onchain_confirmed_sats": 0, "onchain_pending_sats": 0,
                                 "deposit_needed_sats": None, "deadline": 1900000000, "asp_connected": True, "two_step_on": True, "allowed": False,
                                 "refusal": "The wallet cannot work out the fee yet for some of your coins. This page asks the server to refresh them automatically.",
                                 "unpriceable": True, "preparing_since": None, "in_progress_since": None, "live": [], "claimable": False, "destination": None}
                if path == "/admin/exit/prepare":
                    return 200, {"started": True}
                return base(path, req, calls)
            return h

        def c_unpriceable(p, pr, calls):
            open_exit(p)
            p.wait_for_timeout(3500)  # the page re-reads after asking for the refresh
            want(pr, "says why the fee is missing", p.inner_text("#ex-refusal"), "cannot work out the fee yet")
            want(pr, "hover explains more", p.get_attribute("#ex-refusal", "title") or "", "Lightning")
            if count(calls, "POST", "/admin/exit/prepare") != 1: pr.append("the refresh should be asked for once by itself, not on every refresh of the page")
            if not p.is_disabled("#ex-start"): pr.append("Start must stay grey while the fee cannot be worked out")

        ok &= run_case(pw, "emergency withdrawal: unpriceable coins are refreshed automatically, with a hover explanation", unpriceable_handler(), c_unpriceable)
    sys.exit(0 if ok else 1)


main()
