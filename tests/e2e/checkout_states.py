#!/usr/bin/env python3
"""Checkout page states, with the server mocked (no ebp binary needed).

Loads assets/checkout.html in a phone-sized browser and feeds it every answer the
server can give (pending, paid, overpaid, underpaid, expired, cancelled, on-chain
waiting, not found, connection errors, hostile text) and checks what the customer
sees: the right message, the pay details hidden once a payment is finished, no
script errors, no sideways scrolling.  Run: python3 tests/e2e/checkout_states.py
"""
import json, os, sys, time
from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "checkout.html")).read()
QR = open(os.path.join(ROOT, "assets", "qr.js")).read()
ORIGIN = "http://ebp.test"
X = "<img src=x onerror=window.__xss=1>"


def invoice(**kw):
    d = {"return_url": "https://shop.example/order/7", "store_name": "Test Shop", "status": "pending",
         "amount_sats": 1500, "fiat_currency": "USD", "fiat_amount": "1.00",
         "ark_address": "tark1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq",
         "expires_at_unix": int(time.time()) + 600, "onchain_enabled": False, "onchain_address": None,
         "bip321": "bitcoin:?ark=tark1qqqq&amount=0.000015", "lightning": None, "test_mode": True}
    d.update(kw)
    return d


def run_case(pw, name, json_resp, statuses, checks, width=360):
    """json_resp: dict, 404, or 'abort'. statuses: list of dicts / codes / 'abort', used in order, last repeats."""
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": width, "height": 780})
    p = ctx.new_page()
    errs = []
    p.on("pageerror", lambda e: errs.append(str(e)))
    seen = {"n": 0}

    def handler(route):
        path = route.request.url[len(ORIGIN):]
        if path == "/assets/qr.js":
            return route.fulfill(status=200, content_type="application/javascript", body=QR)
        if path.endswith("/json"):
            if json_resp == "abort":
                return route.abort()
            if json_resp == 404:
                return route.fulfill(status=404, content_type="application/json", body='{"error":{"message":"not found"}}')
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(json_resp))
        if path.endswith("/status"):
            i = min(seen["n"], len(statuses) - 1)
            seen["n"] += 1
            r = statuses[i]
            if r == "abort":
                return route.abort()
            if isinstance(r, int):
                return route.fulfill(status=r, content_type="text/plain", body="oops")
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(r))
        return route.fulfill(status=200, content_type="text/html", body=HTML)

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/i/inv_test")
    p.wait_for_timeout(1500)
    api = {
        "status": lambda: p.inner_text("#status"),
        "card": lambda: p.is_visible("#pay-card"),
        "back": lambda: p.inner_text("#back-link") if p.is_visible("#back") else "",
        "overflow": lambda: p.evaluate("document.documentElement.scrollWidth - window.innerWidth"),
        "page": p,
    }
    problems = []
    try:
        checks(api, problems)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script errors: {errs}")
    if api["overflow"]() > 0:
        problems.append(f"page scrolls sideways by {api['overflow']()}px")
    if p.evaluate("window.__xss||0"):
        problems.append("hostile text ran as script")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def has(api, problems, what, text, in_=None):
    got = api["status"]() if in_ is None else in_
    if text.lower() not in got.lower():
        problems.append(f"{what}: wanted '{text}', saw '{got[:90]}'")


def main():
    ok = True
    with sync_playwright() as pw:
        def c_pending(a, pr):
            has(a, pr, "waiting message", "Waiting for your payment")
            if not a["card"](): pr.append("pay details must be visible while pending")

        def c_paid(a, pr):
            has(a, pr, "paid message", "Paid. Thank you")
            if a["card"](): pr.append("pay details must go away once paid")
            has(a, pr, "return link", "Done", a["back"]())

        def c_over(a, pr):
            has(a, pr, "overpaid message", "little extra")
            if a["card"](): pr.append("pay details must go away once paid")

        def c_under(a, pr):
            has(a, pr, "underpaid message", "part of your payment")
            if not a["card"](): pr.append("pay details stay while part-paid")

        def wait_back_link(a):
            # The first paint shows "Back to <shop>"; the next status poll adds
            # "to try again". Wait for it instead of guessing a delay (a slow CI
            # machine made a fixed wait flaky).
            try:
                a["page"].wait_for_function(
                    "document.getElementById('back-link').textContent.indexOf('try again') >= 0",
                    timeout=6000)
            except Exception:  # noqa: BLE001  the check below reports what was there
                pass

        def c_expired(a, pr):
            wait_back_link(a)
            has(a, pr, "expired message", "expired")
            if a["card"](): pr.append("pay details must go away once expired")
            has(a, pr, "back link", "try again", a["back"]())

        def c_cancelled(a, pr):
            wait_back_link(a)
            has(a, pr, "cancelled message", "cancelled")
            if a["card"](): pr.append("pay details must go away once cancelled")
            has(a, pr, "back link", "try again", a["back"]())

        def c_chain(a, pr):
            has(a, pr, "waiting for confirmations", "0 of 1 confirmations")

        def c_nf(a, pr):
            has(a, pr, "not found message", "could not find")

        def c_net(a, pr):
            has(a, pr, "connection message", "check your connection")

        def c_recover(a, pr):
            a["page"].wait_for_timeout(4500)
            has(a, pr, "recovers after errors", "Paid. Thank you")

        def c_evil(a, pr):
            if a["back"]() and "javascript" in a["page"].get_attribute("#back-link", "href").lower():
                pr.append("a javascript: link was followed")
            has(a, pr, "shows the store name as text", X, a["page"].inner_text("#store-name"))

        BOLT = "lntbs1500n1pfakeinvoicefakeinvoicefakeinvoice"

        def copied(a):
            """Click Copy and return what would have gone to the clipboard."""
            pg = a["page"]
            pg.evaluate("window.__copied = null; Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: function (t) { window.__copied = t; return Promise.resolve(); } } })")
            pg.click("#copy-btn")
            pg.wait_for_timeout(100)
            return pg.evaluate("window.__copied")

        def shown(a):
            return a["page"].text_content("#address").strip()

        def c_ln_first(a, pr):
            pg = a["page"]
            if shown(a) != BOLT: pr.append(f"Lightning invoice should be shown first, saw {shown(a)[:50]}")
            if copied(a) != BOLT: pr.append("Copy should copy the Lightning invoice")
            has(a, pr, "says which way is shown", "Paying with Lightning", pg.inner_text("#method"))
            if not pg.is_visible("#ln-fee"): pr.append("the Lightning fee note should be visible without opening anything")
            has(a, pr, "fee note", "3 sats", pg.inner_text("#ln-fee"))
            has(a, pr, "fee note says what the wallet will show", "wallet will show 1,503 sats", pg.inner_text("#ln-fee"))
            if pg.get_attribute("#tab-ln", "aria-selected") != "true": pr.append("Lightning tab not selected")
            pg.click("#other summary")
            pg.click("#tab-bip321")
            if not shown(a).startswith("bitcoin:"): pr.append("the combined code should show after choosing it")
            if copied(a) != shown(a): pr.append("Copy should follow the combined code")
            if pg.is_visible("#ln-fee"): pr.append("the Lightning fee note should hide on other methods")
            pg.click("#tab-ark")
            if not shown(a).startswith("tark1"): pr.append("Ark address should show after choosing Ark")
            if copied(a) != shown(a): pr.append("Copy should follow Ark")
            if not pg.is_visible("#ark-amount"): pr.append("Ark must show the 'type this amount' box")
            has(a, pr, "Ark amount box says the sats", "1,500 sats", pg.inner_text("#ark-amount"))
            has(a, pr, "Ark amount box says to type it", "type exactly", pg.inner_text("#ark-amount"))
            if pg.evaluate("parseFloat(getComputedStyle(document.getElementById('ark-sats')).fontSize)") < 30: pr.append("the Ark amount must be big")
            pg.evaluate("window.__copied = null"); pg.click("#copy-ark-amount"); pg.wait_for_timeout(100)
            if pg.evaluate("window.__copied") != "1500": pr.append(f"'Copy the number' should copy 1500, saw {pg.evaluate('window.__copied')}")
            pg.click("#tab-chain")
            if pg.is_visible("#ark-amount"): pr.append("the Ark amount box belongs to Ark only")
            if shown(a) != "tb1qfakefakefakefakefake": pr.append(f"on-chain tab should show the plain address only, saw {shown(a)[:40]}")
            CHAIN_URI = "bitcoin:tb1qfakefakefakefakefake?amount=0.00001500"
            if copied(a) != CHAIN_URI: pr.append(f"Copy should copy the on-chain link with the amount, saw {copied(a)}")
            if pg.get_attribute("#open-wallet", "href") != CHAIN_URI: pr.append("the wallet button and Copy must carry the same on-chain link")
            if not pg.is_visible("#copy-addr"): pr.append("on-chain needs a 'Copy address only' button")
            pg.evaluate("window.__copied = null")
            pg.click("#copy-addr"); pg.wait_for_timeout(100)
            if pg.evaluate("window.__copied") != "tb1qfakefakefakefakefake": pr.append("'Copy address only' should copy just the address")
            if "bitcoin:" in shown(a): pr.append("the on-chain choice must not include the combined code")
            pg.click("#tab-ark")
            if pg.is_visible("#copy-addr"): pr.append("'Copy address only' belongs to on-chain only")
            pg.click("#tab-chain")
            pg.click("#tab-ln")
            if copied(a) != BOLT: pr.append("Copy should follow back to Lightning")

        def c_no_ln(a, pr):
            if not shown(a).startswith("bitcoin:"): pr.append(f"without Lightning the combined code shows first, saw {shown(a)[:40]}")
            if a["page"].is_visible("#tab-ln"): pr.append("no Lightning tab without an invoice")
            if a["page"].is_visible("#ln-fee"): pr.append("no fee note without Lightning")

        def c_usd_sats(a, pr):
            pg = a["page"]
            has(a, pr, "amount shows dollars", "1,000 sats ($0.40)", pg.inner_text("#amount"))
            has(a, pr, "sats line shows dollars", "1,000 sats ($0.40)", pg.inner_text("#sats"))
            has(a, pr, "fee note shows dollars", "3 sats (<$0.01)", pg.inner_text("#ln-fee"))

        def c_usd_shop_in_dollars(a, pr):
            pg = a["page"]
            has(a, pr, "dollar shop heading", "USD 1.00", pg.inner_text("#amount"))
            if "$" in pg.inner_text("#sats"): pr.append("a shop that prices in dollars needs no second dollar figure")

        def c_usd_euro_shop(a, pr):
            has(a, pr, "euro shop sats line", "about 1,500 sats ($1.32)", a["page"].inner_text("#sats"))

        def c_usd_unknown(a, pr):
            pg = a["page"]
            if "$" in pg.inner_text("#amount") or "(" in pg.inner_text("#sats"): pr.append("no dollars when the price is unknown")


        TOPUP = {"status": "underpaid", "amount_sats": 1500, "received_sats": 600, "remaining_sats": 900,
                 "topup_window_open": True, "topup_closes_at_unix": int(time.time()) + 600}
        TOPUP_INV = dict(lightning={"bolt11": BOLT, "fee_estimate_sat": 3}, onchain_enabled=True, onchain_address="tb1qfakefakefakefakefake")

        def c_topup(a, pr):
            pg = a["page"]
            has(a, pr, "part-paid headline", "Part paid: 900 sats to go")
            has(a, pr, "overpay guard", "Don't send the full amount again")
            if not a["card"](): pr.append("pay details stay visible during a top-up")
            has(a, pr, "headline shows only what is missing", "900 sats to go", pg.inner_text("#amount"))
            has(a, pr, "received line", "600 sats of 1,500 sats received", pg.inner_text("#sats"))
            has(a, pr, "countdown is the top-up window", "Time left to send the rest", pg.inner_text("#countdown"))
            if pg.is_visible("#tab-ln") or pg.is_visible("#tab-bip321"): pr.append("Lightning and the one-code link carry the full amount and must be hidden during a top-up")
            if pg.get_attribute("#tab-ark", "aria-selected") != "true": pr.append("Ark should be selected for the top-up")
            if not shown(a).startswith("tark1"): pr.append(f"the Ark address should be shown, saw {shown(a)[:30]}")
            has(a, pr, "top-up Ark box shows only what is still missing", "900 sats", pg.inner_text("#ark-amount"))
            if "1,500" in pg.inner_text("#ark-amount"): pr.append("the top-up box must not show the full price")
            has(a, pr, "says what to type", "type in 900 sats.", pg.inner_text("#method"))
            pg.click("#other summary")
            pg.click("#tab-chain")
            want = "bitcoin:tb1qfakefakefakefakefake?amount=0.00000900"
            if copied(a) != want: pr.append(f"on-chain top-up must carry only the missing amount, saw {copied(a)}")

        def c_topup_lapsed(a, pr):
            has(a, pr, "lapsed headline", "the rest didn't come in time")
            has(a, pr, "reference", "inv_test")
            if a["card"](): pr.append("no pay details once the top-up window has closed")
            if "send" in a["status"]().lower().replace("send exactly", "") and "full amount" in a["status"]().lower(): pr.append("must not ask for more money")
            has(a, pr, "headline back to the total", "1,500 sats", a["page"].inner_text("#amount") + " " + a["page"].inner_text("#sats"))

        def c_offline_expired(a, pr):
            a["page"].wait_for_timeout(3500)
            has(a, pr, "expired while offline", "Don't send money to it")
            if a["card"](): pr.append("a dead QR must not stay payable when we cannot reach the server")

        def c_offline_expired_recovers(a, pr):
            a["page"].wait_for_timeout(5000)
            if not a["card"](): pr.append("the server's answer (still open) must win over the local clock")
            has(a, pr, "waiting text is back", "Waiting for your payment")

        def c_lost(a, pr):
            a["page"].wait_for_timeout(1200)
            has(a, pr, "connection lost", "Can't reach the store's server")
            has(a, pr, "advice", "before sending anything")
            if not a["card"](): pr.append("pay details stay while only the connection is lost")

        def c_lost_clears(a, pr):
            a["page"].wait_for_timeout(5000)
            has(a, pr, "message clears when the server answers", "Waiting for your payment")

        cases = [
            ("pending: waiting message, pay details shown", invoice(), [{"status": "pending", "onchain_confirmations": None}], c_pending),
            ("paid: thank-you, pay details hidden, 'Done' link", invoice(), [{"status": "paid"}], c_paid),
            ("overpaid: thank-you with a note about the extra", invoice(), [{"status": "overpaid"}], c_over),
            ("underpaid: asks the customer to contact the store", invoice(), [{"status": "underpaid"}], c_under),
            ("expired: message, pay details hidden, link back to try again", invoice(), [{"status": "expired"}], c_expired),
            ("cancelled: message, pay details hidden, link back", invoice(), [{"status": "cancelled"}], c_cancelled),
            ("on-chain waiting: shows the confirmation count", invoice(), [{"status": "pending", "onchain_confirmations": 0, "onchain_confirmations_required": 1}], c_chain),
            ("unknown payment: plain 'could not find' message", 404, [{"status": "pending"}], c_nf),
            ("server unreachable: plain connection message", "abort", [{"status": "pending"}], c_net),
            ("status errors then recovery: the page keeps trying and shows paid", invoice(), [500, "abort", {"status": "paid"}], c_recover),
            ("hostile text and a javascript: link are inert", invoice(store_name=X, return_url="javascript:window.__xss=1", fiat_amount=X, amount_sats=10**18), [{"status": "pending"}], c_evil),
        ]
        cases += [
            ("Lightning is shown first; Copy follows every way to pay; on-chain has no combined code",
             invoice(lightning={"bolt11": BOLT, "fee_estimate_sat": 3}, onchain_enabled=True, onchain_address="tb1qfakefakefakefakefake"),
             [{"status": "pending"}], c_ln_first),
            ("no Lightning invoice: the combined code is the default", invoice(), [{"status": "pending"}], c_no_ln),
        ]
        cases += [
            ("sats-only payment shows dollars next to sats (and in the fee note)",
             invoice(fiat_currency=None, fiat_amount=None, amount_sats=1000, usd_per_btc=40000.0, lightning={"bolt11": BOLT, "fee_estimate_sat": 3}),
             [{"status": "pending"}], c_usd_sats),
            ("a shop pricing in dollars does not repeat dollars", invoice(usd_per_btc=40000.0), [{"status": "pending"}], c_usd_shop_in_dollars),
            ("a euro shop still gets the dollar figure", invoice(fiat_currency="EUR", fiat_amount="1.20", amount_sats=1500, usd_per_btc=88000.0), [{"status": "pending"}], c_usd_euro_shop),
            ("no price known: no dollars, nothing broken", invoice(fiat_currency=None, fiat_amount=None, usd_per_btc=None), [{"status": "pending"}], c_usd_unknown),
        ]
        cases += [
            ("underpaid with an open top-up window: part paid, amount missing, overpay guard, only rails that take a top-up",
             invoice(**TOPUP_INV), [TOPUP], c_topup),
            ("underpaid after the window closed: honest message with a reference, no pay details",
             invoice(**TOPUP_INV), [dict(TOPUP, topup_window_open=False)], c_topup_lapsed),
            ("offline past expiry: the QR is hidden and the page says not to pay",
             invoice(expires_at_unix=int(time.time()) - 20), ["abort"], c_offline_expired),
            ("offline past expiry, then the server says still open: the server wins",
             invoice(expires_at_unix=int(time.time()) - 20), ["abort", "abort", "abort", "abort", {"status": "pending"}], c_offline_expired_recovers),
            ("two failed polls: says the connection is lost",
             invoice(), ["abort", "abort", "abort", "abort"], c_lost),
            ("the lost-connection message clears when the server answers again",
             invoice(), ["abort", "abort", "abort", {"status": "pending"}], c_lost_clears),
        ]
        for name, j, st, chk in cases:
            ok &= run_case(pw, name, j, st, chk)
        ok &= run_case(pw, "narrow phone (320 px) fits", invoice(store_name="A very long shop name " * 6), [{"status": "pending"}], c_pending, width=320)
    sys.exit(0 if ok else 1)


main()
