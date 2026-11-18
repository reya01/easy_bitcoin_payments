#!/usr/bin/env python3
"""Setup wizard behaviours, with the server mocked (no ebp binary needed).

Loads assets/setup.html in a phone-sized browser and checks: the restore question
and word counter, the Copy buttons on the one-time secrets, the loading labels on
the slow buttons, the message when the setup code changes, and that error boxes
are announced.  Run: python3 tests/e2e/setup_wizard_states.py
"""
import json, os, sys
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "setup.html")).read()
ORIGIN = "http://ebp.test"
TWELVE = " ".join(f"word{i}" for i in range(1, 13))


def status(**kw):
    d = {"mode": "test", "wallet_created": False, "backup_confirmed": False, "config_written": False,
         "managed": True, "store_count": 0, "setup_complete": False, "wallet_restored": False,
         "setup_code_required": False, "https_required": False, "barkd_token_present": True,
         "mode_chosen": False}
    d.update(kw)
    return d


def run_case(pw, name, handler_for, checks):
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 390, "height": 844})
    p = ctx.new_page()
    errs, posts = [], []
    p.on("pageerror", lambda e: errs.append(str(e)))
    watch_csp(p, errs)

    def handler(route):
        req = route.request
        path = req.url[len(ORIGIN):]
        if path == "/setup":
            return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
        if serve_asset(route, path):
            return
        out = handler_for(path, req, posts)
        if out is None:
            return route.fulfill(status=200, content_type="application/json", body="{}")
        code, body = out
        return route.fulfill(status=code, content_type="application/json", body=json.dumps(body))

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/setup")
    p.wait_for_timeout(500)
    problems = []
    try:
        checks(p, problems, posts)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script errors: {errs}")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def want(problems, what, got, text):
    if text.lower() not in (got or "").lower():
        problems.append(f"{what}: wanted '{text}', saw '{(got or '')[:100]}'")


def main():
    ok = True
    with sync_playwright() as pw:
        # --- restore: question, counter, block until 12 words
        def h_restore(path, req, posts):
            if path == "/setup/status":
                return 200, status()
            if path == "/setup/wallet":
                posts.append(req.post_data_json)
                return 200, {}
            return None

        def c_restore(p, pr, posts):
            p.click("#btn-go-restore")
            want(pr, "question asked in plain words", p.inner_text("#restore-kind"), "Which kind of money did your old server use?")
            if not p.is_checked("#restore-kind-signet"): pr.append("play money should be pre-selected on a test server")
            want(pr, "counter starts at zero", p.inner_text("#restore-count"), "0 of 12 words")
            p.fill("#restore-words", "one two three\nfour  five")
            want(pr, "counter counts words across spaces and line breaks", p.inner_text("#restore-count"), "5 of 12 words")
            p.check("#restore-paper")
            p.click("#btn-restore")
            want(pr, "blocked until 12 words", p.inner_text("#restore-error"), "That's 5 words. Recovery words are always 12")
            if posts: pr.append("nothing may be sent while the count is wrong")
            p.fill("#restore-words", TWELVE)
            want(pr, "counter at twelve", p.inner_text("#restore-count"), "12 of 12 words")
            p.click("#btn-restore")
            p.wait_for_timeout(300)
            if len(posts) != 1 or posts[0].get("network") != "signet": pr.append(f"12 words should be sent once on signet, saw {posts}")
            if p.input_value("#restore-words") != "": pr.append("the words must be wiped after sending")
            want(pr, "counter resets after wipe", p.inner_text("#restore-count"), "0 of 12 words")

        def c_restore_mainnet(p, pr, posts):
            p.click("#btn-go-restore")
            p.check("#restore-kind-mainnet")
            if p.input_value("#network") != "mainnet": pr.append("the answer must drive the Network box")
            if not p.is_visible("#restore-mainnet-box"): pr.append("real money needs its own tick-box on this screen")
            p.fill("#restore-words", TWELVE); p.check("#restore-paper")
            p.click("#btn-restore")
            want(pr, "mainnet must be confirmed", p.inner_text("#restore-error"), "Tick the box")
            if posts: pr.append("nothing sent before the real-money tick-box")
            p.check("#restore-mainnet-ok"); p.click("#btn-restore"); p.wait_for_timeout(300)
            if len(posts) != 1 or posts[0].get("network") != "mainnet" or posts[0].get("mainnet_understood") is not True: pr.append(f"mainnet restore should be sent with the confirmation, saw {posts}")

        def c_restore_back_wipes(p, pr, posts):
            p.click("#btn-go-restore"); p.fill("#restore-words", TWELVE)
            p.click("#btn-restore-back")
            p.click("#btn-go-restore")
            if p.input_value("#restore-words") != "": pr.append("Back must wipe the words")
            want(pr, "counter reset", p.inner_text("#restore-count"), "0 of 12 words")

        ok &= run_case(pw, "restore: plain question, live word counter, blocked until 12 words", h_restore, c_restore)
        ok &= run_case(pw, "restore: choosing real money needs its own confirmation", h_restore, c_restore_mainnet)
        ok &= run_case(pw, "restore: Back wipes the words and the counter", h_restore, c_restore_back_wipes)

        # --- loading labels, error announced, no wallet form flash
        def h_slow(path, req, posts):
            if path == "/setup/status":
                return 200, status()
            if path == "/setup/wallet":
                return 500, {"error": {"message": "boom"}}
            return None

        def c_labels(p, pr, posts):
            p.evaluate("""() => { const f = window.fetch; window.fetch = (u, o) => String(u).includes('/setup/wallet') ? new Promise(r => setTimeout(() => f(u, o).then(r), 600)) : f(u, o); }""")
            p.click("#btn-wallet")
            want(pr, "wallet label while working", p.inner_text("#btn-wallet"), "Creating your wallet")
            if not p.is_disabled("#btn-wallet"): pr.append("button must be disabled while working")
            p.wait_for_timeout(1200)
            want(pr, "label restored", p.inner_text("#btn-wallet"), "Create the wallet")
            want(pr, "error shown", p.inner_text("#wallet-error"), "boom")
            if p.get_attribute("#wallet-error", "role") != "alert": pr.append("error boxes should be announced (role=alert)")

        ok &= run_case(pw, "wallet button shows a loading label, restores it, error is announced", h_slow, c_labels)

        # --- store step: copy buttons on one-time secrets, loading label
        def h_store(path, req, posts):
            if path == "/setup/status":
                return 200, status(wallet_created=True, backup_confirmed=True, config_written=True, mode_chosen=True)
            if path == "/setup/check-payout":
                return 200, {"verdict": "verified", "message": "Verified: ok"}
            if path == "/setup/store":
                return 200, {"api_key": "ebp_test_KEY123", "webhook_secret": "whsec_SECRET456"}
            return None

        def c_secrets(p, pr, posts):
            p.fill("#store", "Shop"); p.fill("#admin-pw", "correct horse battery staple")
            p.select_option("#payout-kind", "ark")
            p.fill("#payout", "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh")
            p.dispatch_event("#payout", "change"); p.wait_for_timeout(200)
            p.click("#btn-store"); p.wait_for_timeout(500)
            want(pr, "shown-once warning", p.inner_text("#store-secrets"), "Shown once.")
            want(pr, "shown-once warning says what happens", p.inner_text("#store-secrets"), "gone for good")
            p.evaluate("window.__c = []; Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: t => { window.__c.push(t); return Promise.resolve(); } } })")
            p.click("[data-copy=api-key]"); p.click("[data-copy=webhook-secret]"); p.wait_for_timeout(150)
            got = p.evaluate("window.__c")
            if got != ["ebp_test_KEY123", "whsec_SECRET456"]: pr.append(f"Copy buttons should copy exactly the two secrets, saw {got}")
            want(pr, "button confirms", p.inner_text("[data-copy=api-key]"), "Copied")
            if p.inner_text("#api-key").strip() != "ebp_test_KEY123": pr.append("the secret text itself must stay clean (no button text inside it)")
            p.click("#btn-done")
            if p.inner_text("#api-key").strip() != "": pr.append("secrets must be wiped after Continue")

        ok &= run_case(pw, "one-time secrets: shown-once line and Copy buttons", h_store, c_secrets)

        # --- setup code changed: plain explanation
        def h_expired(path, req, posts):
            if path == "/setup/status":
                return 200, status(wallet_created=True, backup_confirmed=True, config_written=True, setup_code_required=True)
            if path == "/setup/verify-code":
                return 200, {}
            if path == "/setup/check-payout":
                return 200, {"verdict": "verified", "message": "Verified: ok"}
            if path == "/setup/store":
                return 401, {"error": "setup_token_required", "message": "setup code needed"}
            return None

        def c_expired(p, pr, posts):
            p.fill("#code", "ABCD-EFGH-JKLM"); p.click("#btn-code"); p.wait_for_timeout(300)
            p.fill("#store", "Shop"); p.fill("#admin-pw", "correct horse battery staple")
            p.select_option("#payout-kind", "ark")
            p.fill("#payout", "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh")
            p.dispatch_event("#payout", "change"); p.wait_for_timeout(200)
            p.click("#btn-store"); p.wait_for_timeout(500)
            if not p.is_visible("#code"): pr.append("should go back to the code screen")
            t = p.inner_text("#code-error")
            want(pr, "explains the code changed", t, "no longer valid")
            want(pr, "progress saved", t, "progress is saved")
            want(pr, "says where to look", t, "payment server's Console")
            if p.get_attribute("#code-error", "role") != "alert": pr.append("the message should be announced")

        ok &= run_case(pw, "setup code changed mid-way: explained, progress is saved, says where to look", h_expired, c_expired)

        # --- recovery words note in plain words, technical detail folded away
        def h_backup(path, req, posts):
            if path == "/setup/status":
                return 200, status(wallet_created=True, managed=False)
            return None

        def c_note(p, pr, posts):
            note = p.inner_text("#words-ceremony-note")
            want(pr, "plain voice", note, "only copy of your money")
            if p.is_visible("#words-ceremony-note details p"): pr.append("technical detail should be folded away")
            if "--expose-mnemonic" in p.inner_text("#words-manual"): pr.append("the line under the button should not mention barkd flags")
            p.click("#words-ceremony-note summary")
            want(pr, "technical friend detail", p.inner_text("#words-ceremony-note"), "--expose-mnemonic")

        ok &= run_case(pw, "recovery-word note: plain words up front, mechanism in a folded 'technical friend' box", h_backup, c_note)

        # --- restored wallet: reassurance that is true
        def h_restored(path, req, posts):
            if path == "/setup/status":
                return 200, status(wallet_created=True, backup_confirmed=True, config_written=True, wallet_restored=True)
            return None

        def c_restored(p, pr, posts):
            t = p.inner_text("#step-restorefile")
            want(pr, "reassurance", t, "your coins are safe")
            want(pr, "true advice about the network", t, "make a new server")
            if "tap back" in t.lower(): pr.append("must not tell people to press Back: the network is fixed once the wallet exists")

        ok &= run_case(pw, "restored wallet: balance-zero reassurance does not give impossible advice", h_restored, c_restored)
    sys.exit(0 if ok else 1)


main()
