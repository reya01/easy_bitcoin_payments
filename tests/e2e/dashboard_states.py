#!/usr/bin/env python3
"""Admin page behaviours, with the server mocked (no ebp binary needed).

Checks: the in-page confirmation on destructive buttons (and that nothing happens
until it is confirmed), the "Password changed" message, the "Forgot your password?"
link, retry buttons when a card cannot load, real links to the guides, and that
problem messages are announced.  Run: python3 tests/e2e/dashboard_states.py
"""
import json, os, sys
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

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
    watch_csp(p, errs)
    p.add_init_script("window.confirm = function () { window.__nativeConfirm = true; return false; };")
    # The test address is plain http, where browsers hide passkeys; stand in for them,
    # and answer "add" the way a cancelled prompt does.
    p.add_init_script("""
      window.PublicKeyCredential = window.PublicKeyCredential || function () {};
      if (!navigator.credentials) {
        var buf = function (n) { return new Uint8Array([n, n + 1]).buffer; };
        Object.defineProperty(navigator, "credentials", { configurable: true, value: {
          create: function () { return Promise.reject(Object.assign(new Error("test"), { name: "NotAllowedError" })); },
          get: function () {
            window.__pkGets = (window.__pkGets || 0) + 1;
            if (window.__pkCancel) return Promise.reject(Object.assign(new Error("test"), { name: "NotAllowedError" }));
            return Promise.resolve({ id: "AAAA", response: { clientDataJSON: buf(1), authenticatorData: buf(3), signature: buf(5) } });
          } } });
      }""")

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        if path in ("/admin", "/admin/"):
            return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
        if serve_asset(route, path):
            return
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

        def resend_handler(path, req, calls):
            if path == "/api/v1/webhook-deliveries":
                return 200, [
                    {"id": "whd_f", "event": "invoice.paid", "status": "failed", "attempts": 8, "created_at": "2026-11-17T10:00:00Z"},
                    {"id": "whd_p", "event": "invoice.paid", "status": "pending", "attempts": 0, "created_at": "2026-11-17T10:01:00Z"},
                ]
            if path == "/api/v1/webhook-deliveries/whd_f/resend":
                return 200, {"queued": True}
            return signed_in(path, req, calls)

        def c_resend(p, pr, calls):
            p.click('#tabs button[data-v="hooks"]')
            p.wait_for_timeout(300)
            buttons = p.locator("#deliveries button:text-is('Resend')")
            if buttons.count() != 1: pr.append(f"only the failed notice should offer Resend, saw {buttons.count()}")
            buttons.first.click(); p.wait_for_timeout(300)
            if count(calls, "POST", "/api/v1/webhook-deliveries/whd_f/resend") != 1: pr.append("Resend should send once")
            want(pr, "confirmation", p.inner_text("#deliveries-msg"), "Queued")

        ok &= run_case(pw, "Resend queues a failed notification and says so", resend_handler, c_resend)

        def resend_fails(path, req, calls):
            if path == "/api/v1/webhook-deliveries":
                return 200, [{"id": "whd_f", "event": "invoice.paid", "status": "failed", "attempts": 8, "created_at": "2026-11-17T10:00:00Z"}]
            if path == "/api/v1/webhook-deliveries/whd_f/resend":
                return 500, {"error": "internal error"}
            return signed_in(path, req, calls)

        def c_resend_fails(p, pr, calls):
            p.click('#tabs button[data-v="hooks"]')
            p.wait_for_timeout(300)
            p.click("#deliveries button:text-is('Resend')"); p.wait_for_timeout(300)
            want(pr, "error shown", p.inner_text("#deliveries-msg"), "Could not queue it")
            if not p.is_enabled("#deliveries button:text-is('Resend')"): pr.append("the button should work again after an error")

        ok &= run_case(pw, "a failed Resend says so and can be tried again", resend_fails, c_resend_fails)

        def upkeep_handler(path, req, calls):
            if path == "/api/v1/admin/settlements":
                if "group=upkeep" in (req.url or ""):
                    return 200, [
                        {"kind": "renew", "amount_sats": 995, "status": "failed", "detail": "the coins were not taken into a round; they will be renewed again", "destination": None, "created_at": "2026-11-17T09:27:48Z"},
                        {"kind": "renew", "amount_sats": 754, "status": "renewed", "detail": None, "destination": None, "created_at": "2026-11-15T20:06:47Z"},
                    ]
                return 200, []
            return signed_in(path, req, calls)

        def c_upkeep(p, pr, calls):
            p.wait_for_timeout(300)
            txt = p.inner_text("#upkeep")
            want(pr, "reason shown for the failed line", txt, "Why: the coins were not taken into a round")
            if txt.lower().count("why:") != 1: pr.append(f"only the failed line should say why, saw: {txt}")

        ok &= run_case(pw, "failed upkeep lines say why", upkeep_handler, c_upkeep)
        def off_handler(path, req, calls):
            if path == "/admin/session":
                return 200, {"authenticated": True, "csrf": "tok", "password_set": True, "twofa": True, "paused": False}
            if path == "/admin/2fa/status":
                return 200, {"recovery_left": 10}
            return signed_in(path, req, calls)

        def c_one_off(p, pr, calls):
            p.click('#tabs button[data-v="set"]'); p.wait_for_timeout(300)
            p.evaluate("var d = document.getElementById('twofa-card').closest('details'); if (d) d.open = true;"); p.wait_for_timeout(200)
            card = p.locator("#twofa-card")
            if card.get_by_role("button", name="Turn off app code", exact=True).count() != 1: pr.append("before opening: expected one Turn off app code button")
            card.get_by_role("button", name="Turn off app code", exact=True).click(); p.wait_for_timeout(200)
            names = [b.inner_text() for b in card.locator("button:visible").all()]
            offs = [n for n in names if n.lower().startswith("turn off")]
            if offs != ["Turn off app code"]: pr.append(f"after opening: expected exactly one turn-off button, saw {names}")
            card.get_by_role("button", name="Cancel").click(); p.wait_for_timeout(200)
            if card.get_by_role("button", name="Turn off app code", exact=True).count() != 1 or not card.get_by_role("button", name="Turn off app code", exact=True).is_visible(): pr.append("Cancel should bring the first button back")
            if card.locator("#tf-off-pw").count(): pr.append("Cancel should close the form")

        ok &= run_case(pw, "turning off two-step sign-in shows one Turn off button at a time", off_handler, c_one_off)

        ok &= run_case(pw, "destructive buttons ask in the page first, and nothing happens until confirmed", hooks_handler, c_confirms)

        # ---- passkeys (beta) in the two-step card
        KEY = {"id": 1, "name": "Bitwarden on my laptop", "synced": True, "created_at": 1700000000, "last_used_at": None}
        OPTIONS = {"challenge": "AAAA", "rp": {"id": "pay.example.com", "name": "Easy Bitcoin Payments"},
                   "user": {"id": "AAAA", "name": "Shop", "displayName": "Shop"}, "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
                   "timeout": 120000, "attestation": "none", "authenticatorSelection": {"userVerification": "preferred"}, "excludeCredentials": []}

        GET_OPTIONS = {"challenge": "AAAA", "rpId": "pay.example.com", "timeout": 120000, "userVerification": "preferred",
                       "allowCredentials": [{"type": "public-key", "id": "AAAA"}]}

        def passkey_handler(keys, unavailable=None, twofa=True):
            cur = list(keys)

            def h(path, req, calls):
                if path == "/admin/session":
                    return 200, {"authenticated": True, "csrf": "tok", "password_set": True, "twofa": twofa, "paused": False, "passkeys": bool(cur)}
                if path == "/admin/2fa/status":
                    return 200, {"enabled": twofa, "recovery_left": 9, "passkeys": len(cur)}
                if path == "/admin/passkeys/status":
                    return 200, {"two_step_on": twofa, "passkeys": cur, "max": 10, "unavailable": unavailable}
                if path == "/admin/passkeys/begin":
                    return 200, {"options": OPTIONS}
                if path == "/admin/passkeys/stepup/begin":
                    return 200, {"options": GET_OPTIONS}
                if path == "/admin/passkeys/stepup/finish":
                    return 200, {"ticket": "TICKET123"}
                if path == "/admin/passkeys/disable":
                    cur.clear()
                    return 200, {"passkeys": []}
                return signed_in(path, req, calls)
            return h

        def open_settings(p):
            p.click('#tabs button[data-v="set"]'); p.wait_for_timeout(300)
            p.evaluate("var d = document.getElementById('twofa-card').closest('details'); if (d) d.open = true;"); p.wait_for_timeout(300)

        def c_pk_list(p, pr, calls):
            open_settings(p)
            card = p.locator("#twofa-card"); txt = card.inner_text()
            for w in ("Authenticator app code", "Passkeys (beta)", "Bitwarden on my laptop", "kept in a password manager", "not used yet",
                      "What is the difference?", "To turn the app code off, first turn off your passkeys"):
                want(pr, "card shows", txt, w)
            for b in ("Turn off app code", "Add a passkey", "Turn off passkeys", "Remove"):
                if card.get_by_role("button", name=b, exact=True).count() != 1: pr.append(f"expected one '{b}' button")
            card.get_by_role("button", name="Turn off passkeys", exact=True).click(); p.wait_for_timeout(200)
            if not p.is_visible("#pk-pw") or not p.is_visible("#pk-code"): pr.append("turning passkeys off asks for the password and a code")
            want(pr, "explains what stays", p.inner_text("#pk-box"), "Your app code stays on")
            card.get_by_role("button", name="Cancel", exact=True).last.click(); p.wait_for_timeout(200)
            card.get_by_role("button", name="Remove", exact=True).click(); p.wait_for_timeout(200)
            want(pr, "remove names the passkey", p.inner_text("#pk-box"), "Remove Bitwarden on my laptop?")
            if p.is_visible("#pk-name"): pr.append("removing does not ask for a name")
            card.get_by_role("button", name="Cancel", exact=True).last.click(); p.wait_for_timeout(200)
            want(pr, "list comes back", p.inner_text("#pk-box"), "Bitwarden on my laptop")

        ok &= run_case(pw, "passkeys: the list, both turn-off choices and the remove form", passkey_handler([KEY]), c_pk_list)

        def c_pk_add(p, pr, calls):
            open_settings(p)
            card = p.locator("#twofa-card")
            card.get_by_role("button", name="Add a passkey", exact=True).click(); p.wait_for_timeout(200)
            for i in ("#pk-pw", "#pk-code", "#pk-name"):
                if not p.is_visible(i): pr.append(f"the add form needs {i}")
            p.fill("#pk-pw", "a good long password"); p.fill("#pk-code", "123456"); p.fill("#pk-name", "My phone")
            card.get_by_role("button", name="Add passkey", exact=True).click(); p.wait_for_timeout(500)
            if count(calls, "POST", "/admin/passkeys/begin") != 1: pr.append("adding should ask the server to begin once")
            if count(calls, "POST", "/admin/passkeys/finish"): pr.append("a cancelled prompt must not save anything")
            want(pr, "cancelled prompt message", p.inner_text("#pk-msg"), "cancelled or timed out")
            if not card.get_by_role("button", name="Add passkey", exact=True).is_enabled(): pr.append("the button should work again after a cancelled prompt")

        ok &= run_case(pw, "passkeys: a cancelled browser prompt says so and saves nothing", passkey_handler([]), c_pk_add)

        def c_pk_none(p, pr, calls):
            open_settings(p)
            card = p.locator("#twofa-card"); txt = card.inner_text()
            want(pr, "empty state", txt, "None yet")
            if card.get_by_role("button", name="Turn off passkeys", exact=True).count(): pr.append("nothing to turn off yet")
            if card.get_by_role("button", name="Add a passkey", exact=True).count() != 1: pr.append("one Add a passkey button")
            if p.is_visible("#twofa-card .hidden"): pr.append("hidden note should stay hidden with no passkeys")

        ok &= run_case(pw, "passkeys: none yet offers only Add", passkey_handler([]), c_pk_none)

        def c_pk_unavailable(p, pr, calls):
            open_settings(p)
            txt = p.locator("#twofa-card").inner_text()
            want(pr, "reason", txt, "cannot be used on this server's address")
            want(pr, "reason detail", txt, "web address with a name")
            if p.locator("#twofa-card").get_by_role("button", name="Add a passkey").count(): pr.append("no Add button when passkeys cannot work here")

        ok &= run_case(pw, "passkeys: a server address that cannot hold them says why", passkey_handler([], unavailable="Passkeys need this server to have a web address with a name"), c_pk_unavailable)

        def c_pk_off_state(p, pr, calls):
            open_settings(p)
            txt = p.locator("#twofa-card").inner_text()
            want(pr, "tells when passkeys come", txt, "once the app code is on")
            if p.locator("#pk-box").count(): pr.append("no passkey section while the app code is off")

        ok &= run_case(pw, "passkeys: not offered until the app code is on", passkey_handler([], twofa=False), c_pk_off_state)

        def c_pk_stepup(p, pr, calls):
            open_settings(p)
            card = p.locator("#twofa-card")
            card.get_by_role("button", name="Turn off passkeys", exact=True).click(); p.wait_for_timeout(200)
            if card.get_by_role("button", name="Use a passkey instead").count() != 1: pr.append("the code box should offer a passkey")
            card.get_by_role("button", name="Use a passkey instead").click(); p.wait_for_timeout(500)
            if count(calls, "POST", "/admin/passkeys/stepup/begin") != 1 or count(calls, "POST", "/admin/passkeys/stepup/finish") != 1:
                pr.append("approving should ask the server to begin and finish once")
            fin = [c for c in calls if c[1] == "/admin/passkeys/stepup/finish"]
            if fin:
                for k in ("challenge", "client_data_json", "authenticator_data", "signature", '"id":"AAAA"'):
                    if k not in (fin[0][2] or ""): pr.append(f"finish body lacks {k}")
            want(pr, "approved note", card.inner_text(), "Passkey approved")
            if p.get_attribute("#pk-code", "type") != "password": pr.append("the ticket should be hidden as dots")
            p.fill("#pk-pw", "a good long password")
            card.get_by_role("button", name="Turn off passkeys", exact=True).last.click(); p.wait_for_timeout(400)
            sent = [c for c in calls if c[1] == "/admin/passkeys/disable"]
            if len(sent) != 1 or '"code":"pk:TICKET123"' not in (sent[0][2] or ""): pr.append(f"the action should carry the ticket as its code, saw {sent}")
            if p.locator("#pk-code").count(): pr.append("the form should be gone after turning passkeys off")
            if p.locator(".pkuse").count(): pr.append("no passkeys left, so no passkey buttons")

        ok &= run_case(pw, "passkeys: a code box offers a passkey and sends its ticket in place of a code", passkey_handler([KEY]), c_pk_stepup)

        def c_pk_typing(p, pr, calls):
            open_settings(p)
            card = p.locator("#twofa-card")
            card.get_by_role("button", name="Turn off passkeys", exact=True).click(); p.wait_for_timeout(200)
            card.get_by_role("button", name="Use a passkey instead").click(); p.wait_for_timeout(400)
            p.click("#pk-code"); p.keyboard.type("1"); p.wait_for_timeout(100)
            if p.get_attribute("#pk-code", "type") == "password": pr.append("typing over an approval should show the box as a normal code box again")
            if p.is_visible("#twofa-card .pknote"): pr.append("the approved note should go when the user types a code instead")
            p.evaluate("window.__pkCancel = true")
            p.fill("#pk-code", "")
            card.get_by_role("button", name="Use a passkey instead").click(); p.wait_for_timeout(400)
            want(pr, "cancel message", card.inner_text(), "cancelled or timed out")
            if p.input_value("#pk-code") != "": pr.append("a cancelled passkey must leave the box empty")

        ok &= run_case(pw, "passkeys: typing a code replaces an approval, and a cancelled prompt leaves the box empty", passkey_handler([KEY]), c_pk_typing)

        def login_pk_handler(offer):
            def h(path, req, calls):
                if path == "/admin/session":
                    return 200, {"authenticated": False, "password_set": True}
                if path == "/admin/login":
                    return 200, {"code_required": True, "passkey": GET_OPTIONS if offer else None}
                if path == "/admin/login/passkey":
                    return 200, {"authenticated": True, "csrf": "tok"}
                return signed_in(path, req, calls)
            return h

        def c_login_pk(p, pr, calls):
            p.fill("#pw", "a good long password"); p.click("#login-btn"); p.wait_for_timeout(400)
            if not p.is_visible("#code-box"): pr.append("the code box should show")
            if not p.is_visible("#login-pk-btn"): pr.append("a passkey button should show under the code box")
            p.click("#login-pk-btn"); p.wait_for_timeout(600)
            sent = [c for c in calls if c[1] == "/admin/login/passkey"]
            if len(sent) != 1 or "client_data_json" not in (sent[0][2] or ""): pr.append(f"the passkey answer should be sent once, saw {sent}")
            if p.is_visible("#v-login"): pr.append("should be signed in")
            if p.is_visible("#login-pk-btn"): pr.append("the passkey button should be gone once signed in")

        ok &= run_case(pw, "sign-in: a passkey can stand in for the code", login_pk_handler(True), c_login_pk)

        def c_login_no_pk(p, pr, calls):
            p.fill("#pw", "a good long password"); p.click("#login-btn"); p.wait_for_timeout(400)
            if not p.is_visible("#code-box"): pr.append("the code box should show")
            if p.is_visible("#login-pk-btn"): pr.append("no passkeys, so no passkey button")

        ok &= run_case(pw, "sign-in: no passkey button when there are no passkeys", login_pk_handler(False), c_login_no_pk)

        def c_login_pk_cancel(p, pr, calls):
            p.evaluate("window.__pkCancel = true")
            p.fill("#pw", "a good long password"); p.click("#login-btn"); p.wait_for_timeout(400)
            p.click("#login-pk-btn"); p.wait_for_timeout(500)
            want(pr, "cancel text", p.inner_text("#login-err"), "cancelled or timed out")
            if count(calls, "POST", "/admin/login/passkey"): pr.append("nothing is sent when the prompt is cancelled")
            if not p.is_visible("#code-box"): pr.append("the code box should stay so a code can be typed")
            if not p.is_enabled("#login-pk-btn"): pr.append("the passkey button should work again")

        ok &= run_case(pw, "sign-in: a cancelled passkey prompt says so and lets the user type a code", login_pk_handler(True), c_login_pk_cancel)

        def pw_handler(path, req, calls):
            if path == "/admin/password":
                return 200, {}
            if path == "/admin/logout":
                return 200, {}
            return signed_in(path, req, calls)

        def c_pw(p, pr, calls):
            p.click('#tabs button[data-v="set"]')
            p.evaluate("document.querySelectorAll('#v-set details.fold').forEach(function(d){d.open=true})")
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
                    return 200, {"min_exit_sats": 50000, "ark_sats": 500000, "fee": {"broadcast_sat": 894, "claim_sat": 28, "total_sat": 922, "txs_to_broadcast": 3},
                                 "onchain_confirmed_sats": confirmed, "onchain_pending_sats": pending, "deposit_needed_sats": needed, "deadline": 1900000000,
                                 "asp_connected": True, "two_step_on": True, "allowed": True, "refusal": None, "in_progress_since": None, "live": [],
                                 "claimable": False, "destination": None, "opted_in": True, "reserve_floor_sats": 5000}
                if path == "/admin/exit/deposit-address":
                    return 200, {"address": TB}
                return signed_in(path, req, calls)
            return h

        def open_exit(p):
            p.click('#tabs button[data-v="set"]')
            p.evaluate("document.querySelectorAll('#v-set details.fold').forEach(function(d){d.open=true})")
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

        def fundable_handler():
            base = exit_handler(confirmed=1500, needed=0)
            def h(path, req, calls):
                if path == "/admin/exit/status":
                    code, body = base(path, req, calls)
                    body["fundable"] = False
                    return code, body
                return base(path, req, calls)
            return h

        def c_unfundable(p, pr, calls):
            open_exit(p)
            p.fill("#ex-dest", TB); p.fill("#ex-pw", "pw"); p.fill("#ex-code", "123456"); p.check("#ex-ok")
            want(pr, "says the wallet does not count it", p.inner_text("#ex-fee-verdict"), "Not usable yet")
            want(pr, "checklist says so", p.inner_text("#ex-checks"), "does not count it as usable")
            if not p.is_disabled("#ex-start"): pr.append("Start must stay grey while the wallet says it cannot fund the withdrawal")

        ok &= run_case(pw, "emergency withdrawal: the wallet's own cannot-fund warning keeps Start grey", fundable_handler(), c_unfundable)

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
                    return 200, {"min_exit_sats": 50000, "ark_sats": 500000, "fee": None, "onchain_confirmed_sats": 0, "onchain_pending_sats": 0,
                                 "deposit_needed_sats": None, "deadline": 1900000000, "asp_connected": True, "two_step_on": True, "allowed": False,
                                 "refusal": "The wallet cannot work out the fee yet for some of your coins. This page asks the server to refresh them automatically.",
                                 "unpriceable": True, "preparing_since": None, "in_progress_since": None, "live": [], "claimable": False, "destination": None, "opted_in": True, "reserve_floor_sats": 5000}
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

        def preset_handler(switched_off=False, mode="test", confirmed=0, pending=0, target=5000):
            base = exit_handler(mode=mode)
            def h(path, req, calls):
                if path == "/admin/exit/status":
                    return 200, {"min_exit_sats": 50000, "ark_sats": 5000, "fee": {"broadcast_sat": 300, "claim_sat": 50, "total_sat": 350, "txs_to_broadcast": 1},
                                 "onchain_confirmed_sats": confirmed, "onchain_pending_sats": pending, "deposit_needed_sats": 0, "deadline": 1900000000,
                                 "asp_connected": True, "two_step_on": True, "allowed": False, "refusal": "Not worth it right now.", "unpriceable": False,
                                 "switched_off": switched_off, "reserve_min_sats": 1500, "reserve_floor_sats": 5000, "reserve_target_sats": target, "reserve_high_sats": 10000, "opted_in": True, "in_progress_since": None, "live": [], "claimable": False, "destination": None}
                return base(path, req, calls)
            return h

        def c_preset(p, pr, calls):
            open_exit(p)
            want(pr, "heading", p.inner_text("#ex-deposit"), "Keep fee money on the server")
            want(pr, "the floor", p.inner_text("#ex-fee-rows"), "The server keeps at least")
            want(pr, "the floor amount", p.inner_text("#ex-fee-rows"), "5,000")
            want(pr, "the recommendation range", p.inner_text("#ex-fee-rows"), "10,000")
            want(pr, "still to send", p.inner_text("#ex-fee-rows"), "Still to send")
            want(pr, "asks for the floor, not optional", p.inner_text("#ex-fee-verdict"), "Please send at least 5,000")
            want(pr, "says why", p.inner_text("#ex-fee-verdict"), "turned on")
            want(pr, "address shows", p.inner_text("#ex-addr"), TB)
            want(pr, "hover explains the sweep promise", p.get_attribute("#ex-deposit-need", "title") or "", "never touch this money")
            if not p.is_disabled("#ex-start") if p.is_visible("#ex-start") else False: pr.append("Start must stay grey")

        ok &= run_case(pw, "emergency withdrawal: while on, the server asks for 5,000 sats of fee money", preset_handler(), c_preset)

        def c_preset_done(p, pr, calls):
            open_exit(p)
            want(pr, "done", p.inner_text("#ex-fee-verdict"), "Set aside")
            if p.is_visible("#ex-addr-box"): pr.append("no address needed once it is set aside")
            if count(calls, "POST", "/admin/exit/deposit-address") != 0: pr.append("no address should be fetched when none is needed")
            if p.is_visible("#ex-left-box"): pr.append("at the 5,000 floor there is nothing above it to send back")
            want(pr, "send-back explained (hidden at the floor)", p.inner_text("#ex-left-box"), "stays here while the emergency withdrawal is turned on")

        ok &= run_case(pw, "emergency withdrawal: fee money already set aside", preset_handler(confirmed=5000), c_preset_done)

        def c_preset_min(p, pr, calls):
            open_exit(p)
            want(pr, "keeping enough", p.inner_text("#ex-fee-verdict"), "The server is keeping enough")
            want(pr, "still suggests more", p.inner_text("#ex-fee-verdict"), "safer")

        ok &= run_case(pw, "emergency withdrawal: above the 5,000 floor but below the recommendation", preset_handler(confirmed=6000, target=8000), c_preset_min)

        def c_preset_below(p, pr, calls):
            open_exit(p)
            want(pr, "asks for the rest", p.inner_text("#ex-fee-verdict"), "Please send at least 3,000")
            want(pr, "state line warns", p.inner_text("#exo-state"), "please add")

        ok &= run_case(pw, "emergency withdrawal: below the 5,000 floor asks for the rest", preset_handler(confirmed=2000), c_preset_below)

        def c_off(p, pr, calls):
            open_exit(p)
            if p.is_visible("#ex-deposit"): pr.append("no fee-money prompt where the withdrawal is switched off")

        ok &= run_case(pw, "emergency withdrawal: switched off on real money shows no fee-money prompt", preset_handler(switched_off=True, mode="live"), c_off)

        # ---- the emergency withdrawal is opt-in; what is kept; the send minimum with its hard floors
        def pay_view(kind="ark", **over):
            d = {"address": ARK if kind == "ark" else TB, "kind": kind,
                 "sweep_min_sats": 1500 if kind == "ark" else 50000, "sweep_min_floor_sats": 1500 if kind == "ark" else 50000,
                 "sweep_min_is_custom": False, "ark_min_floor_sats": 1500, "onchain_min_floor_sats": 50000,
                 "keep_sats": 0, "keep_is_custom": False, "keep_min_sats": 1, "keep_max_sats": 10000000,
                 "exit_opted_in": False, "exit_min_sats": 50000, "small_coin_sats": 1300, "live": False, "set_by_server_config": False}
            d.update(over)
            return d

        def pay_handler(kind="ark", opted=False, two_step=True, confirmed=0, running=False, **over):
            base = exit_handler(confirmed=confirmed, needed=0)
            def h(path, req, calls):
                if path == "/api/v1/payout":
                    return 200, pay_view(kind, exit_opted_in=opted, **over)
                if path == "/admin/payout/min":
                    b = json.loads(req.post_data)
                    return 200, pay_view(kind, sweep_min_sats=b["min_sats"] or (1500 if kind == "ark" else 50000), sweep_min_is_custom=b["min_sats"] is not None)
                if path == "/admin/payout/cap":
                    b = json.loads(req.post_data)
                    return 200, pay_view(kind, exit_opted_in=True, keep_sats=b["cap_sats"] or 0, keep_is_custom=b["cap_sats"] is not None)
                if path == "/admin/exit/optin":
                    b = json.loads(req.post_data)
                    return 200, {"opted_in": b["on"], "message": "Turned on." if b["on"] else "Turned off."}
                if path == "/admin/exit/status":
                    code, body = base(path, req, calls)
                    body["opted_in"] = opted
                    body["two_step_on"] = two_step
                    if running:
                        body["in_progress_since"] = 1900000000 - 3600
                        body["live"] = [{"vtxo_id": "abcdef0123456789", "state": "processing", "words": "sending its transactions"}]
                    return code, body
                return base(path, req, calls)
            return h

        def open_payout(p):
            p.click('#tabs button[data-v="set"]'); p.wait_for_timeout(300)
            p.evaluate("document.getElementById('fold-payout').open = true"); p.wait_for_timeout(500)

        def posted(calls, path):
            return json.loads([c for c in calls if c[1] == path][-1][2])

        def c_min_ark(p, pr, calls):
            open_payout(p)
            if not p.is_visible("#min-box"): pr.append("the send-minimum box should show")
            for w in ("at least 1,500 sats", "(the default, which is also the lowest we allow)", "sends all of the money"):
                want(pr, "now line", p.inner_text("#min-now"), w)
            for w in ("1,300 sats", "all of it at once"):
                want(pr, "why line", p.inner_text("#min-why"), w)
            want(pr, "manual note", p.inner_text("#sweep-box"), "under 1,300 sats")
            if p.is_visible("#min-reset"): pr.append("no reset button on the default")
            # Under the floor: refused on the page with a plain reason, nothing is sent.
            # Typed and Save clicked straight away (the box loses focus on that click: nothing
            # may move the button from under it).
            p.fill("#min-pw", "pw"); p.fill("#min-new", "1000"); p.click("#min-go"); p.wait_for_timeout(400)
            for w in ("1,000 sats is lower than the smallest amount we allow", "1,500 sats", "get stuck", "Please type 1,500 sats or more"):
                want(pr, "floor refusal", p.inner_text("#min-msg"), w)
            if count(calls, "POST", "/admin/payout/min"): pr.append("nothing is sent under the floor")
            p.fill("#min-new", "1499"); p.wait_for_timeout(1000)
            want(pr, "explained while typing", p.inner_text("#min-usd"), "lower than the smallest amount we allow")
            # Exactly the floor and above save, as the Ark kind.
            p.fill("#min-new", "3000"); p.wait_for_timeout(1000)
            if p.inner_text("#min-usd"): pr.append("an allowed Ark amount must not warn")
            p.click("#min-go"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/payout/min")
            if b.get("kind") != "ark" or b.get("min_sats") != 3000 or b.get("admin_password") != "pw": pr.append(f"wrong body {b}")
            want(pr, "saved", p.inner_text("#min-msg"), "Saved")

        ok &= run_case(pw, "settings: an Ark payout address sends everything at 1,500 sats or more, with a hard floor", pay_handler("ark"), c_min_ark)

        def c_min_chain(p, pr, calls):
            open_payout(p)
            want(pr, "now line", p.inner_text("#min-now"), "at least 50,000 sats")
            for w in ("recommend much more", "network fee that can be high and changes a lot", "Ark payout address is almost free"):
                want(pr, "why line", p.inner_text("#min-why"), w)
            p.fill("#min-pw", "pw"); p.fill("#min-new", "49999"); p.click("#min-go"); p.wait_for_timeout(400)
            for w in ("49,999 sats is lower than the smallest amount we allow for on-chain sends", "50,000 sats", "network fee", "Please type 50,000 sats or more", "Ark payout address"):
                want(pr, "floor refusal", p.inner_text("#min-msg"), w)
            if count(calls, "POST", "/admin/payout/min"): pr.append("nothing is sent under the floor")
            p.fill("#min-new", "60000"); p.wait_for_timeout(1000)
            want(pr, "low warning", p.inner_text("#min-usd"), "We recommend much more")
            p.click("#min-go"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/payout/min")
            if b.get("kind") != "onchain" or b.get("min_sats") != 60000: pr.append(f"wrong body {b}")
            p.fill("#min-new", "500000"); p.wait_for_timeout(1000)
            if p.inner_text("#min-usd"): pr.append("a high amount must not warn")

        ok &= run_case(pw, "settings: an on-chain payout address has a hard 50,000 floor, a warning and an explanation", pay_handler("onchain"), c_min_chain)

        def c_min_custom(p, pr, calls):
            open_payout(p)
            want(pr, "custom line", p.inner_text("#min-now"), "at least 250,000 sats")
            want(pr, "custom line", p.inner_text("#min-now"), "(your setting)")
            if not p.is_visible("#min-reset"): pr.append("a reset button should show for a custom value")
            p.fill("#min-pw", "pw"); p.click("#min-reset"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/payout/min")
            if b.get("min_sats") is not None or b.get("kind") != "onchain": pr.append(f"reset must send null for its kind: {b}")

        ok &= run_case(pw, "settings: a custom send minimum can be reset to the default", pay_handler("onchain", sweep_min_sats=250000, sweep_min_is_custom=True), c_min_custom)

        def c_optin_off(p, pr, calls):
            open_exit(p)
            if not p.is_visible("#ex-off-box"): pr.append("the turned-off panel should show")
            if p.is_visible("#ex-on-box"): pr.append("the turned-on panel must stay hidden")
            for sel in ("#ex-deposit", "#ex-start-box", "#ex-running", "#ex-left-box"):
                if p.is_visible(sel): pr.append(f"{sel} must stay hidden while it is off")
            if p.inner_text("#ex-facts"): pr.append("no exit figures while it is off")
            for w in ("turned off", "5,000 sats of regular Bitcoin", "50,000 sats are there", "everything is sent to your payout address"):
                want(pr, "off text", p.inner_text("#ex-off-box"), w)
            if p.is_visible("#exo-twostep"): pr.append("the two-step note is only for when it is off")
            p.fill("#exo-pw", "pw"); p.fill("#exo-code", "123456"); p.click("#exo-on"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/exit/optin")
            if b != {"on": True, "admin_password": "pw", "code": "123456"}: pr.append(f"wrong body {b}")
            want(pr, "reply shown", p.inner_text("#exo-msg"), "Turned on")

        ok &= run_case(pw, "emergency withdrawal: it is opt-in, off until the owner turns it on", pay_handler("ark", opted=False), c_optin_off)

        def c_optin_no2fa(p, pr, calls):
            open_exit(p)
            if not p.is_visible("#exo-twostep"): pr.append("without two-step sign-in the page should say it is needed")
            want(pr, "note", p.inner_text("#exo-twostep"), "two-step sign-in")

        ok &= run_case(pw, "emergency withdrawal: without two-step sign-in the opt-in says why it cannot be used yet", pay_handler("ark", opted=False, two_step=False), c_optin_no2fa)

        def c_off_leftover(p, pr, calls):
            open_exit(p)
            if not p.is_visible("#ex-left-box"): pr.append("while it is off the fee money can be taken back")
            if not p.is_visible("#ex-auth"): pr.append("the address, password and code boxes should show for it")
            want(pr, "explained", p.inner_text("#ex-left-box"), "takes back all of the fee money")
            want(pr, "button", p.inner_text("#ex-left"), "Send leftover to my address")

        ok &= run_case(pw, "emergency withdrawal: turned off, the fee money on the server can be sent out", pay_handler("ark", opted=False, confirmed=7000), c_off_leftover)

        def c_running_while_off(p, pr, calls):
            open_exit(p)
            if not p.is_visible("#ex-running"): pr.append("a withdrawal already running always shows")
            if p.is_visible("#ex-off-box"): pr.append("no turn-on panel while one is running")

        ok &= run_case(pw, "emergency withdrawal: one that is running shows whatever the switch says", pay_handler("ark", opted=False, running=True), c_running_while_off)

        def c_optin_on(p, pr, calls):
            open_exit(p)
            open_payout(p)
            p.evaluate("document.getElementById('ex-box').open = true"); p.wait_for_timeout(700)
            if not p.is_visible("#ex-on-box"): pr.append("the turned-on panel should show")
            if p.is_visible("#ex-off-box"): pr.append("the turn-on panel must be hidden")
            want(pr, "state", p.inner_text("#exo-state"), "keeps at least 5,000 sats of regular Bitcoin")
            want(pr, "keep line", p.inner_text("#cap-now"), "keeps nothing")
            # 7,000 confirmed while on: what is above the 5,000 floor can be sent back.
            if not p.is_visible("#ex-left-box"): pr.append("the part above 5,000 can be sent back while on")
            want(pr, "send-back button", p.inner_text("#ex-left"), "Send back what is above 5,000 sats")
            want(pr, "send-back note", p.inner_text("#ex-left-note"), "At least 5,000 sats stays here")
            # An amount under what an exit needs is allowed, with a plain warning.
            p.fill("#cap-new", "10000"); p.wait_for_timeout(1000)
            want(pr, "warns", p.inner_text("#cap-usd"), "under 50,000 sats")
            p.fill("#cap-pw", "pw"); p.click("#cap-go"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/payout/cap")
            if b.get("cap_sats") != 10000: pr.append(f"wrong body {b}")
            want(pr, "saved", p.inner_text("#cap-msg"), "keeps")
            p.fill("#cap-new", "80000"); p.wait_for_timeout(1000)
            if "under 50,000" in p.inner_text("#cap-usd"): pr.append("80,000 must not warn")
            # Typed and Save clicked straight away: the click must still land.
            n_caps = count(calls, "POST", "/admin/payout/cap")
            p.fill("#cap-pw", "pw"); p.fill("#cap-new", "20000"); p.click("#cap-go"); p.wait_for_timeout(500)
            if count(calls, "POST", "/admin/payout/cap") != n_caps + 1: pr.append("Save clicked right after typing must still save")
            # Turning it off asks first; confirming sends it.
            p.fill("#exoff-pw", "pw"); p.click("#exo-off-go")
            want(pr, "confirm title", p.inner_text("#dlg-title"), "Turn off the emergency withdrawal?")
            p.click("#dlg-back button:text-is(\"Keep it on\")"); p.wait_for_timeout(100)
            if count(calls, "POST", "/admin/exit/optin"): pr.append("nothing is sent before confirming")
            p.click("#exo-off-go"); p.click("#dlg-back .btns button:last-child"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/exit/optin")
            if b.get("on") is not False or b.get("admin_password") != "pw": pr.append(f"wrong body {b}")

        ok &= run_case(pw, "emergency withdrawal: turned on, it keeps fee money, can keep some Ark money, and can be turned off", pay_handler("ark", opted=True, confirmed=7000), c_optin_on)

        def c_keep_custom(p, pr, calls):
            open_exit(p)
            want(pr, "keep line", p.inner_text("#cap-now"), "keeps")
            want(pr, "keep line", p.inner_text("#cap-now"), "60,000 sats")
            want(pr, "keep line", p.inner_text("#cap-now"), "(your setting)")
            want(pr, "min line mentions it", p.inner_text("#min-now"), "not counting the")
            if not p.is_visible("#cap-reset"): pr.append("a button to keep nothing should show")
            p.fill("#cap-pw", "pw"); p.click("#cap-reset"); p.wait_for_timeout(500)
            b = posted(calls, "/admin/payout/cap")
            if b.get("cap_sats") is not None: pr.append(f"keeping nothing must send null: {b}")

        ok &= run_case(pw, "emergency withdrawal: a kept amount is shown and can be set back to nothing", pay_handler("ark", opted=True, confirmed=7000, keep_sats=60000, keep_is_custom=True), c_keep_custom)

        # --- optional first test payment on Home
        def tp_handler(paid=False, mode="live"):
            def h(path, req, calls):
                if path == "/api/v1/admin/status":
                    return 200, {"checks": [], "counters": {}, "wallet": {}, "mode": mode, "usd_per_btc": 67000}
                if path == "/api/v1/invoices":
                    return 200, ([{"id": "x", "status": "paid"}] if paid and "status=paid" in req.url else [])
                return signed_in(path, req, calls)
            return h

        def c_tp_shown(p, pr, calls):
            p.wait_for_timeout(300)
            if not p.is_visible("#test-prompt"): pr.append("the optional test-payment card should show before any payment"); return
            want(pr, "title says optional", p.inner_text("#test-prompt"), "not done yet (optional)")
            want(pr, "says 1,000 sats", p.inner_text("#test-prompt"), "1,000 sats")
            want(pr, "says Lightning", p.inner_text("#test-prompt"), "Lightning")
            want(pr, "real money explained", p.inner_text("#test-prompt"), "real money")
            p.click("#test-prompt button:text-is(\"Try a test payment\")"); p.wait_for_timeout(200)
            if not p.is_visible("#v-orders"): pr.append("Try should open the Payments tab")
            if p.input_value("#tp-sats") != "1000": pr.append("amount should be 1000")
            p.click('#tabs button[data-v="home"]'); p.wait_for_timeout(300)
            p.click("#test-prompt button:text-is(\"Skip for now\")")
            if p.is_visible("#test-prompt"): pr.append("Skip should hide the card")
            p.reload(); p.wait_for_timeout(500)
            if p.is_visible("#test-prompt"): pr.append("a skipped card should stay hidden on reload")

        ok &= run_case(pw, "test payment: optional card, try and skip", tp_handler(), c_tp_shown)

        def c_tp_done(p, pr, calls):
            p.wait_for_timeout(300)
            if p.is_visible("#test-prompt"): pr.append("no test-payment card once a payment has been paid")

        ok &= run_case(pw, "test payment: hidden once a payment is paid", tp_handler(paid=True), c_tp_done)

        def c_tp_play(p, pr, calls):
            p.wait_for_timeout(300)
            want(pr, "play money wording", p.inner_text("#test-prompt"), "play money")

        ok &= run_case(pw, "test payment: play-money wording on a test server", tp_handler(mode="test"), c_tp_play)

        # --- weekly wallet check: a new server waits a week
        def recon_handler(due, last, nxt):
            def h(path, req, calls):
                if path == "/admin/reconcile/status":
                    return 200, {"paid_orders": 2, "swept_sats": 0, "due": due, "days": 7, "paid_sats": 2000, "last_checked_at": last, "next_due_at": nxt}
                return signed_in(path, req, calls)
            return h

        def c_recon_wait(p, pr, calls):
            p.wait_for_timeout(300)
            t = p.inner_text("#recon")
            want(pr, "says the first check is not due yet", t, "your first one is due")
            want(pr, "offers a check now", t, "Tap to check now")
            if "Time for your weekly wallet check" in t: pr.append("a new server must not be told it is time yet")
            if "done" in t.lower(): pr.append("must not say 'done' when no check has happened")

        ok &= run_case(pw, "weekly wallet check: not due in a server's first week", recon_handler(False, None, 1900000000), c_recon_wait)

        def c_recon_due(p, pr, calls):
            p.wait_for_timeout(300)
            want(pr, "due card", p.inner_text("#recon"), "Time for your weekly wallet check")

        ok &= run_case(pw, "weekly wallet check: due after a week", recon_handler(True, None, 1700000000), c_recon_due)

        # --- a slow start-up session answer must not undo a Pause click
        def pause_race(path, req, calls):
            if path == "/admin/session":
                return 200, {"authenticated": True, "csrf": "x", "twofa": False, "paused": False}
            if path == "/admin/pause":
                return 200, {"paused": True}
            return signed_in(path, req, calls)

        def c_pause_race(p, pr, calls):
            # Hold the next session check (opening Settings starts one), click Pause,
            # then let the old answer in.
            p.evaluate("""() => { const real = window.fetch; window.__held = false; window.__release = null;
              window.fetch = function (u, o) {
                if (String(u).indexOf('/admin/session') >= 0 && !window.__held) {
                  window.__held = true;
                  return new Promise(function (res) { window.__release = function () { res(real.call(window, u, o)); }; });
                }
                return real.call(window, u, o);
              }; }""")
            p.click('#tabs button[data-v="set"]')
            p.evaluate("document.querySelectorAll('#v-set details.fold').forEach(function(d){d.open=true})")
            p.wait_for_function("() => window.__held === true", timeout=5000)
            p.get_by_role("button", name="Pause new payments").click()
            p.wait_for_function("() => document.getElementById('pause-card').textContent.indexOf('Paused') >= 0", timeout=5000)
            p.evaluate("window.__release && window.__release()")
            p.wait_for_timeout(600)
            if "Paused" not in p.inner_text("#pause-card"): pr.append("a late session answer must not undo the owner's Pause click")

        ok &= run_case(pw, "pause: a late start-up answer cannot undo the click", pause_race, c_pause_race)


        # --- phone alerts: "Set up" must visibly lead somewhere
        def phone_handler(fail):
            state = {"s": "undecided"}
            def h(path, req, calls):
                if path == "/admin/alerts/status":
                    return 200, {"state": state["s"], "server": "https://ntfy.sh", "default_server": "https://ntfy.sh", "topic": "ebp-faketopic123", "activity": "daily"}
                if path == "/admin/alerts/enable":
                    if fail: return 500, {"error": {"message": "Could not reach the alerts server."}}
                    state["s"] = "on"; return 200, {"ok": True}
                if path == "/admin/alerts/disable":
                    state["s"] = "off"; return 200, {"ok": True}
                return signed_in(path, req, calls)
            return h

        def c_phone_ok(p, pr, calls):
            p.wait_for_timeout(300)
            if not p.is_visible("#phone-home"): pr.append("the alerts question should show on Home"); return
            p.click("#phone-home button:text-is(\"Set up phone alerts\")"); p.wait_for_timeout(600)
            if not p.is_visible("#v-alerts"): pr.append("Set up should take the owner to the Alerts tab, where the steps are")
            if p.locator("#v-set #phone-set").count(): pr.append("phone alerts must not also sit in Settings")
            if not p.is_visible("#phone-set"): pr.append("the phone alerts section should be opened for the owner")
            if not p.is_visible("#phone-topic"): pr.append("the topic name should be visible after Set up")
            want(pr, "steps are shown", p.inner_text("#phone-set"), "install the free")
            if p.locator("#phone-set img").count(): pr.append("no QR code in the phone-alert steps (a scanner cannot open ntfy:// links)")
            want(pr, "tap link kept", p.inner_text("#phone-set"), "open in ntfy")

        def c_phone_fail(p, pr, calls):
            p.wait_for_timeout(300)
            p.click("#phone-home button:text-is(\"Set up phone alerts\")"); p.wait_for_timeout(500)
            if not p.is_visible("#phone-home"): pr.append("on failure the card must stay so the owner can retry")
            want(pr, "failure explained", p.inner_text("#phone-home"), "Could not reach the alerts server")
            if p.is_disabled("#phone-home button:text-is(\"Set up phone alerts\")"): pr.append("the button must work again after a failure")

        ok &= run_case(pw, "phone alerts: Set up leads to the steps on the Alerts tab", phone_handler(False), c_phone_ok)
        ok &= run_case(pw, "phone alerts: a failed Set up says so and can be retried", phone_handler(True), c_phone_fail)

        # --- Settings: every section is one folded row: a title and one sentence
        def c_settings_folded(p, pr, calls):
            p.click('#tabs button[data-v="set"]'); p.wait_for_timeout(500)
            folds = p.query_selector_all("#v-set details.fold")
            if len(folds) != 8: pr.append(f"expected 8 folded sections, saw {len(folds)}")
            for d in folds:
                name = d.get_attribute("id") or "?"
                if d.evaluate("e => e.open"): pr.append("every section must start folded: " + name)
                if not d.query_selector("summary .ft").inner_text().strip(): pr.append("a folded row needs a title: " + name)
                if not d.query_selector("summary .fs").inner_text().strip().endswith("."): pr.append("a folded row needs a one-sentence summary: " + name)
                if d.query_selector("summary .fo") or d.query_selector("summary .fc"): pr.append("no Expand/Collapse words any more, the whole row opens it: " + name)
                if d.evaluate("e => e.previousElementSibling && e.previousElementSibling.tagName === 'H2' && e.previousElementSibling.textContent !== 'Settings'"): pr.append("the title must be inside the row, not a separate heading: " + name)
            if p.locator("#v-set #phone-set").count(): pr.append("phone alerts belong on the Alerts tab, not in Settings")
            if p.is_visible("#pause-card") or p.is_visible("#pw-save") or p.is_visible("#bk-go"):
                pr.append("section contents must be hidden until opened")
            row = p.evaluate("() => document.querySelector('#fold-pause > summary').getBoundingClientRect().height")
            if row > 120: pr.append(f"a folded row should be compact, it is {row}px tall")
            p.click("#fold-pause > summary"); p.wait_for_timeout(200)
            if not p.is_visible("#pause-card"): pr.append("clicking the row should show the section")
            want(pr, "the sentence stays on the row", p.inner_text("#fold-pause > summary"), "Stops customers")
            if p.is_visible("#pw-save"): pr.append("opening one section must not open the others")
            p.click("#fold-pause > summary"); p.wait_for_timeout(200)
            if p.is_visible("#pause-card"): pr.append("clicking again should hide it")
            p.click('#tabs button[data-v="alerts"]'); p.wait_for_timeout(400)
            if not p.is_visible("#phone-set"): pr.append("the Alerts tab should show Phone alerts open, with no folding")
            if p.locator("#phone-set").evaluate("e => !!e.closest('details')"): pr.append("Phone alerts on the Alerts tab must not be inside a fold")

        ok &= run_case(pw, "settings: eight compact folded rows with one sentence each, phone alerts open on the Alerts tab", signed_in, c_settings_folded)

        # --- late and unmatched payments: money that arrived for an order that
        # could no longer take it must be visible, explained and markable.
        def unmatched_handler(path, req, calls, state={}):
            if path == "/api/v1/unmatched-payments":
                done = "2026-11-17T09:00:00Z" if state.get("handled") else None
                return 200, [
                    {"id": "unm_1", "invoice_id": "inv_1", "order_id": "1042", "rail": "lightning", "amount_sats": 2500,
                     "reason": "invoice_expired", "detected_at": "2026-11-16T10:00:00Z", "resolved_at": done},
                    {"id": "unm_2", "invoice_id": "inv_2", "order_id": None, "rail": "ark", "amount_sats": 700,
                     "reason": "invoice_closed", "detected_at": "2026-11-15T10:00:00Z", "resolved_at": "2026-11-16T09:00:00Z"},
                ]
            if path == "/api/v1/unmatched-payments/unm_1/resolve":
                state["handled"] = True
                return 200, {"resolved": True}
            if path == "/api/v1/admin/status":
                n = 0 if state.get("handled") else 1
                return 200, {"checks": [], "counters": {"unmatched_payments_open": n}, "wallet": {}, "mode": "test", "usd_per_btc": 67000}
            return signed_in(path, req, calls)

        def c_unmatched(p, pr, calls):
            p.wait_for_timeout(300)
            want(pr, "Home counts the payment that needs the owner", p.inner_text("#attention"), "Payments that need you")
            if "1" not in p.inner_text("#alert-badge"): pr.append("the Alerts tab should show a badge of 1")
            p.click('#tabs button[data-v="alerts"]'); p.wait_for_timeout(400)
            text = p.inner_text("#unmatched")
            want(pr, "amount shown", text, "2,500 sats")
            want(pr, "rail shown", text, "Lightning")
            want(pr, "order shown", text, "order 1042")
            want(pr, "reason explained", text, "after the payment window closed")
            want(pr, "the other payment is shown as handled", text, "handled")
            buttons = p.query_selector_all("#unmatched button:text-is(\"Mark as handled\")")
            if len(buttons) != 1: pr.append(f"only the open payment gets a Mark as handled button, saw {len(buttons)}")
            if count(calls, "POST", "/api/v1/unmatched-payments/unm_1/resolve"): pr.append("nothing may be marked handled before the click")
            p.click("#unmatched button:text-is(\"Mark as handled\")"); p.wait_for_timeout(500)
            if count(calls, "POST", "/api/v1/unmatched-payments/unm_1/resolve") != 1: pr.append("one click should mark it handled once")
            if p.query_selector_all("#unmatched button:text-is(\"Mark as handled\")"): pr.append("no button should be left once everything is handled")

        ok &= run_case(pw, "late payments: counted on Home, listed and explained in Alerts, can be marked handled", unmatched_handler, c_unmatched)
        # --- the second step: a code is only ever asked for while two-step
        # sign-in is on, so nobody without it meets a box they cannot fill in.
        def second_step_handler(twofa, paused=False, alerts="on"):
            def h(path, req, calls):
                if path == "/admin/session":
                    return 200, {"authenticated": True, "csrf": "tok", "password_set": True, "twofa": twofa, "paused": paused}
                body = json.loads(req.post_data) if req.post_data else {}
                needs_code = path in ("/admin/keylock/save", "/admin/alerts/disable") or (path == "/admin/pause" and not body.get("paused"))
                if twofa and needs_code and not body.get("code"):
                    return 400, {"error": {"message": "Enter the 6-digit code from your authenticator app."}}
                if path == "/admin/pause": return 200, {"paused": body.get("paused")}
                if path in ("/admin/keylock/save", "/admin/keylock/status"):
                    return 200, {"addresses": [], "last_blocked_address": None, "last_blocked_at": None}
                if path == "/admin/alerts/status":
                    return 200, {"state": alerts, "server": "https://ntfy.sh", "default_server": "https://ntfy.sh", "topic": "ebp-faketopic123", "activity": "daily"}
                if path == "/admin/alerts/disable": return 200, {"ok": True}
                return signed_in(path, req, calls)
            return h

        def last_body(calls, path):
            return json.loads([c for c in calls if c[1] == path][-1][2] or "{}")

        def open_folds(p):
            p.evaluate("document.querySelectorAll('#v-set details.fold, #v-alerts details.fold, #keylock-box').forEach(function(d){d.open=true})")

        def c_resume_off(p, pr, calls):
            p.wait_for_timeout(500)
            if not p.is_visible("#paused-card"): pr.append("the paused banner should show on Home"); return
            if p.is_visible("#paused-card input"): pr.append("no code box while two-step sign-in is off")
            p.click('#paused-card button:text-is("Resume payments")'); p.wait_for_timeout(400)
            if p.is_visible("#paused-card"): pr.append("Resume must work without a code when two-step sign-in is off")

        def c_resume_on(p, pr, calls):
            p.wait_for_timeout(500)
            if not p.is_visible("#paused-card input"): pr.append("the code box should show while two-step sign-in is on"); return
            p.click('#paused-card button:text-is("Resume payments")'); p.wait_for_timeout(400)
            want(pr, "asked for the code, on Home where the button is", p.inner_text("#paused-card"), "6-digit code")
            if not p.is_visible("#paused-card"): pr.append("still paused after a refused Resume")
            if p.is_disabled('#paused-card button:text-is("Resume payments")'): pr.append("the button must work again after a refusal")
            p.fill("#paused-card input", "123456"); p.click('#paused-card button:text-is("Resume payments")'); p.wait_for_timeout(400)
            if p.is_visible("#paused-card"): pr.append("Resume should work with the code")
            if last_body(calls, "/admin/pause").get("code") != "123456": pr.append("the code must be sent with Resume")

        ok &= run_case(pw, "second step: Resume asks for no code without two-step sign-in", second_step_handler(False, paused=True), c_resume_off)
        ok &= run_case(pw, "second step: Resume asks for the code, on Home, with two-step sign-in", second_step_handler(True, paused=True), c_resume_on)

        def c_pause_free(p, pr, calls):
            p.click('#tabs button[data-v="set"]'); open_folds(p); p.wait_for_timeout(300)
            if p.is_visible("#pause-card input"): pr.append("Pause is the emergency button: it must never ask for a code")
            p.click('#pause-card button:text-is("Pause new payments")'); p.wait_for_timeout(400)
            want(pr, "paused", p.inner_text("#pause-card"), "Paused")

        ok &= run_case(pw, "second step: Pause never asks for a code, even with two-step sign-in", second_step_handler(True), c_pause_free)

        def keylock_check(twofa):
            def c(p, pr, calls):
                p.click('#tabs button[data-v="hooks"]'); open_folds(p); p.wait_for_timeout(300)
                if p.is_visible("#kl-code") != twofa: pr.append(f"the shop-address code box should be {'shown' if twofa else 'hidden'}")
                p.fill("#kl-addr", "203.0.113.7"); p.click("#kl-save"); p.wait_for_timeout(400)
                if twofa:
                    want(pr, "asked for the code", p.inner_text("#kl-msg"), "6-digit code")
                    p.fill("#kl-code", "654321"); p.click("#kl-save"); p.wait_for_timeout(400)
                    if last_body(calls, "/admin/keylock/save").get("code") != "654321": pr.append("the code must be sent with the address lock")
                want(pr, "saved", p.inner_text("#kl-msg"), "Saved")
            return c

        ok &= run_case(pw, "second step: shop-address lock saves with no code box when two-step sign-in is off", second_step_handler(False), keylock_check(False))
        ok &= run_case(pw, "second step: shop-address lock asks for the code when two-step sign-in is on", second_step_handler(True), keylock_check(True))

        def phone_check(twofa):
            def c(p, pr, calls):
                p.click('#tabs button[data-v="alerts"]'); open_folds(p); p.wait_for_timeout(500)
                if p.is_visible("#phone-code") != twofa: pr.append(f"the alerts code box should be {'shown' if twofa else 'hidden'}")
                p.click('#phone-set button:text-is("Turn phone alerts off")'); p.wait_for_timeout(400)
                if twofa:
                    want(pr, "asked for the code", p.inner_text("#phone-msg"), "6-digit code")
                    if p.is_disabled('#phone-set button:text-is("Turn phone alerts off")'): pr.append("the button must work again after a refusal")
                    p.fill("#phone-code", "111222"); p.click('#phone-set button:text-is("Turn phone alerts off")'); p.wait_for_timeout(400)
                    if last_body(calls, "/admin/alerts/disable").get("code") != "111222": pr.append("the code must be sent when turning alerts off")
                elif count(calls, "POST", "/admin/alerts/disable") != 1:
                    pr.append("turning alerts off must just work without two-step sign-in")
            return c

        ok &= run_case(pw, "second step: turning alerts off needs no code without two-step sign-in", second_step_handler(False), phone_check(False))
        ok &= run_case(pw, "second step: turning working alerts off asks for the code with two-step sign-in", second_step_handler(True), phone_check(True))
    sys.exit(0 if ok else 1)


main()
