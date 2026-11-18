#!/usr/bin/env python3
"""Real-money (mainnet) wording and guards on the setup and admin pages, with the
server mocked (no ebp binary, no network, no money).

Checks what an owner sees when they choose real money: the choice needs its own
tick-box, the banners say REAL MONEY, the address hints name ark.second.tech and
the real-money prefixes (never play-money ones), a failing address shows the danger
box (without the "only testing" line) and cannot be used without the tick, and the
admin Home shows the red lines for a wrong-network wallet or bad payout address and
shows the emergency withdrawal as available but new (its floor and fee rules apply).
Set SHOTS=/some/dir to also save screenshots.  Run: python3 tests/e2e/mainnet_screens.py
"""
import json, os
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SETUP = open(os.path.join(ROOT, "assets", "setup.html")).read()
DASH = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
QR = open(os.path.join(ROOT, "assets", "qr.js")).read()
ORIGIN = "http://ebp.test"
SHOTS = os.environ.get("SHOTS")
ARK_OK = "ark1pu6h30ws3zgf3g9gkzuvpjxsmrsw3u8eqyy3zxfp9ycnjs2f29vkz6t30xqcnyve5x5mrwwpe8ganc0f78aqyzsjrg3z5v36gf99yknzdfe84q52j2d2924srr43da"

# The wording below is the server's (src/payout.rs); the page only shows it.
VERDICTS = {
    "verified": {"verdict": "verified", "message": "Verified: a well-formed Ark address on the same Ark server as this wallet. We cannot see whether you control it, so check it is yours."},
    "unverified": {"verdict": "unverified", "message": "Could not verify: the address is well formed, but we could not compare it with this server's Ark server, so we do not know that it will work. Sending to an address on a different Ark server can lose the money."},
    "failed": {"verdict": "failed", "message": "that is a test-network address, but this server works with real Bitcoin. Use an address that starts with bc1 or ark1. For an Ark address you need a wallet set to Second's Ark server, ark.second.tech: Noah on Android or Arké on iPhone"},
}


def status(**kw):
    d = {"mode": "live", "wallet_created": False, "backup_confirmed": False, "config_written": False,
         "managed": True, "store_count": 0, "setup_complete": False, "wallet_restored": False,
         "setup_code_required": False, "https_required": False, "barkd_token_present": True, "mode_chosen": False}
    d.update(kw)
    return d


def run_case(pw, name, page_path, html, handler_for, checks):
    b = pw.chromium.launch()
    p = b.new_context(viewport={"width": 390, "height": 844}).new_page()
    errs, calls = [], []
    p.on("pageerror", lambda e: errs.append(str(e)))
    watch_csp(p, errs)

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):].split("?")[0]
        if path == page_path:
            return route.fulfill(status=200, content_type="text/html", body=html, headers=HEADERS)
        if serve_asset(route, path):
            return
        if path == "/assets/qr.js":
            return route.fulfill(status=200, content_type="application/javascript", body=QR)
        calls.append((req.method, path, req.post_data))
        out = handler_for(path, req, calls)
        if out is None:
            return route.fulfill(status=200, content_type="application/json", body="{}")
        return route.fulfill(status=out[0], content_type="application/json", body=json.dumps(out[1]))

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + page_path)
    p.wait_for_timeout(500)
    problems = []
    try:
        checks(p, problems, calls)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script errors: {errs}")
    if SHOTS:
        os.makedirs(SHOTS, exist_ok=True)
        p.screenshot(path=os.path.join(SHOTS, name.split(":")[0].replace(" ", "_") + ".png"), full_page=True)
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def want(pr, what, got, text):
    if text.lower() not in (got or "").lower():
        pr.append(f"{what}: wanted '{text}', saw '{(got or '')[:120]}'")


def refuse(pr, what, got, text):
    if text.lower() in (got or "").lower():
        pr.append(f"{what}: must not say '{text}', saw '{(got or '')[:120]}'")


def posts_to(calls, path):
    return [c for c in calls if c[0] == "POST" and c[1] == path]


def main():
    ok = True
    with sync_playwright() as pw:
        # 1. Choosing real money on a server that started as play money.
        def h_wallet(path, req, calls):
            if path == "/setup/status":
                return 200, status(mode="test")
            if path == "/setup/wallet":
                return 500, {"error": {"message": "stop here"}}  # stay on this step
            return None

        def c_wallet(p, pr, calls):
            p.select_option("#network", "mainnet")
            want(pr, "top banner", p.inner_text("#test-banner") if p.query_selector("#test-banner") else p.inner_text(".banner"), "REAL MONEY")
            if not p.is_visible("#mainnet-warn"): pr.append("the REAL MONEY warning must show")
            want(pr, "warning says permanent", p.inner_text("#mainnet-warn"), "cannot be changed later")
            if p.input_value("#asp") != "https://ark.second.tech": pr.append(f"Ark server should switch to Second's mainnet server, saw {p.input_value('#asp')}")
            if not p.is_visible("#mainnet-ok-box"): pr.append("the tick-box must show")
            p.click("#btn-wallet")
            want(pr, "blocked until ticked", p.inner_text("#wallet-error"), "Tick the box")
            if posts_to(calls, "/setup/wallet"): pr.append("nothing may be sent before the tick-box")
            p.check("#mainnet-ok"); p.click("#btn-wallet"); p.wait_for_timeout(300)
            sent = posts_to(calls, "/setup/wallet")
            if len(sent) != 1 or '"mainnet"' not in (sent[0][2] or ""): pr.append(f"should send once, as mainnet, saw {sent}")
            p.select_option("#network", "signet")
            if p.is_checked("#mainnet-ok"): pr.append("going back to play money must clear the tick")
            if p.input_value("#asp") != "https://ark.signet.2nd.dev": pr.append("Ark server should switch back")

        ok &= run_case(pw, "mainnet wallet step: tick-box, banner, Second's server", "/setup", SETUP, h_wallet, c_wallet)

        # 2. Payout step on a real-money server.
        def make_h(verdict):
            def h(path, req, calls):
                if path == "/setup/status":
                    return 200, status(wallet_created=True, backup_confirmed=True, config_written=True, mode_chosen=True)
                if path == "/setup/check-payout":
                    return 200, VERDICTS[verdict]
                if path == "/setup/store":
                    return 200, {"api_key": "ebp_live_KEY", "webhook_secret": "whsec_X"}
                return None
            return h

        def fill(p, addr, kind="ark"):
            p.fill("#store", "Shop"); p.fill("#admin-pw", "correct horse battery staple")
            p.select_option("#payout-kind", kind)
            p.fill("#payout", addr); p.dispatch_event("#payout", "change"); p.wait_for_timeout(250)

        def c_hints(p, pr, calls):
            p.select_option("#payout-kind", "ark")
            h = p.inner_text("#payout-hint")
            for t in ("ark1", "ark.second.tech", "Noah", "Ark"):
                want(pr, "Ark hint", h, t)
            refuse(pr, "Ark hint", h, "tark1"); refuse(pr, "Ark hint", h, "signet"); refuse(pr, "Ark hint", h, "play-money")
            if p.get_attribute("#payout", "placeholder") != "ark1…": pr.append("placeholder should be ark1…")
            p.select_option("#payout-kind", "onchain")
            h = p.inner_text("#payout-hint")
            want(pr, "on-chain hint", h, "bc1"); refuse(pr, "on-chain hint", h, "tb1"); refuse(pr, "on-chain hint", h, "signet")
            want(pr, "mode banner", p.inner_text(".banner"), "REAL MONEY")

        ok &= run_case(pw, "mainnet payout step: hints name ark.second.tech and bc1, never play money", "/setup", SETUP, make_h("verified"), c_hints)

        def c_verified(p, pr, calls):
            fill(p, ARK_OK)
            want(pr, "green line", p.inner_text("#payout-verdict"), "Verified")
            want(pr, "tells them to check it is theirs", p.inner_text("#payout-verdict"), "check it is yours")
            if p.is_visible("#payout-risk"): pr.append("no danger box for a verified address")

        ok &= run_case(pw, "mainnet payout step: a verified address shows the green line", "/setup", SETUP, make_h("verified"), c_verified)

        def c_unverified(p, pr, calls):
            fill(p, ARK_OK)
            want(pr, "amber line", p.inner_text("#payout-verdict"), "Could not verify")
            want(pr, "says it can lose money", p.inner_text("#payout-verdict"), "can lose the money")
            if p.is_visible("#payout-risk"): pr.append("could-not-verify is a warning, not the danger box")

        ok &= run_case(pw, "mainnet payout step: could-not-verify is an amber warning", "/setup", SETUP, make_h("unverified"), c_unverified)

        def c_failed(p, pr, calls):
            fill(p, "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh")
            if not p.is_visible("#payout-risk"): pr.append("the danger box must show")
            t = p.inner_text("#payout-risk")
            want(pr, "danger", t, "could be lost for good"); want(pr, "names the server", t, "ark.second.tech")
            want(pr, "names the wallets", t, "Noah"); want(pr, "names the wallets", t, "Arké")
            refuse(pr, "real-money danger box", t, "only testing with play money")
            want(pr, "refusal starts with a capital and ends with a full stop", t, "good. That is a test-network address")
            want(pr, "no run-on into the next sentence", t, "Arké on iPhone. Bitcoin payments cannot be undone")
            p.click("#btn-store"); p.wait_for_timeout(300)
            want(pr, "blocked", p.inner_text("#store-error"), "failed a check")
            if posts_to(calls, "/setup/store"): pr.append("a failing address must not be saved without the tick")
            p.check("#payout-risk-ok"); p.click("#btn-store"); p.wait_for_timeout(400)
            sent = posts_to(calls, "/setup/store")
            if len(sent) != 1 or '"accept_risky_payout": true' not in (sent[0][2] or "").replace('":true', '": true'): pr.append(f"ticked: one request flagged as accepted risk, saw {sent}")

        ok &= run_case(pw, "mainnet payout step: a failing address is blocked until ticked, with no 'only testing' line", "/setup", SETUP, make_h("failed"), c_failed)

        # 3. Admin Home on a real-money server.
        def h_home(wallet_net_bad=False, payout_bad=False):
            def h(path, req, calls):
                if path == "/admin/session":
                    return 200, {"authenticated": True, "csrf": "tok", "password_set": True}
                if path == "/api/v1/admin/status":
                    checks = [{"id": "https", "level": "ok", "title": "Your address is secure (HTTPS)", "detail": "x"}]
                    if wallet_net_bad:
                        checks.append({"id": "network", "level": "bad", "title": "This server is set to real money, but its wallet is play money", "detail": "The wallet was made for a test network, so real payments cannot work here."})
                    if payout_bad:
                        checks.append({"id": "payout", "level": "bad", "title": "Your payout address is not valid", "detail": "Transfers to your own wallet are stopped until this is fixed."})
                    return 200, {"checks": checks, "counters": {}, "wallet": {"offchain_sats": 1200}, "mode": "live", "usd_per_btc": 67000, "version": "0.2.0"}
                if path == "/admin/exit/status":
                    return 200, {"allowed": False, "two_step_on": True, "refusal": "Not worth it right now. There is less than 50000 sats on the server (1200 sats), and taking it out on your own costs about 300 sats in network fees and a day or more.", "live": [], "deposit_needed_sats": 0}
                return None
            return h

        def c_home(p, pr, calls):
            want(pr, "banner", p.inner_text("#mode-banner"), "REAL MONEY")
            refuse(pr, "banner", p.inner_text("#mode-banner"), "play money")
            want(pr, "wrong network line", p.inner_text("#health"), "set to real money, but its wallet is play money")
            want(pr, "bad payout line", p.inner_text("#health"), "payout address is not valid")
            p.click('#tabs button[data-v="set"]'); p.evaluate("document.getElementById('ex-box').open = true; document.getElementById('ex-box').dispatchEvent(new Event('toggle'))")
            p.wait_for_timeout(500)
            want(pr, "the folded section says it is new on real money", p.evaluate("document.getElementById('ex-box').textContent"), "only been tried in small tests")
            ex = p.evaluate("document.getElementById('ex-refusal') ? document.getElementById('ex-refusal').textContent : ''")
            want(pr, "emergency withdrawal says why it will not start now", ex, "less than 50000 sats")
            refuse(pr, "emergency withdrawal is no longer called switched off", ex, "not switched on")
            if p.evaluate("document.getElementById('ex-start').disabled") is not True: pr.append("the Start button must stay disabled")

        ok &= run_case(pw, "mainnet admin Home: banner, red lines, emergency withdrawal available with its floor", "/admin", DASH, h_home(True, True), c_home)

    print("ALL PASS" if ok else "SOME FAILED")
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
