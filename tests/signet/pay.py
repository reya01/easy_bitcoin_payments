#!/usr/bin/env python3
"""Signet payment drill (needs coins). Real EBP container + its real managed
barkd + a SECOND barkd inside the same container acting as the payer wallet.

You fund the payer wallet by sending signet coins to the address this script
prints (and writes to the job summary). Then it pays EBP's invoices for real:
exact / under / over / expired / Lightning / server restart mid-payment,
captures barkd's real answers, and returns leftovers to RETURN_TO (if set).

Env: RETURN_TO (optional address), MIN_SATS (default 20000), WAIT_MIN (default 55).
The payer wallet's recovery words are never printed or stored: when the job
ends the machine is destroyed, so only signet test coins are ever at stake.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.request

from playwright.sync_api import sync_playwright

BASE = "http://localhost:8080"
ADMIN_PW = "signet drill pay password"
RETURN_TO = os.environ.get("RETURN_TO", "").strip()
MIN_SATS = int(os.environ.get("MIN_SATS", "20000") or 20000)
WAIT_MIN = int(os.environ.get("WAIT_MIN", "55") or 55)
REBUILD = os.environ.get("REBUILD_PAYER", "") == "true"
results, notes = [], []
state = {}


def sh(cmd, inp=None, timeout=180):
    r = subprocess.run(cmd, shell=True, input=inp, capture_output=True, text=True, timeout=timeout)
    return r.returncode, (r.stdout + r.stderr)


def wait_for(fn, what, timeout=90, every=1.0):
    end = time.time() + timeout
    last = None
    while time.time() < end:
        try:
            v = fn()
            if v:
                return v
        except Exception as e:  # noqa: BLE001
            last = e
        time.sleep(every)
    raise AssertionError(f"timed out waiting for {what} ({last!r})")


class Abort(Exception):
    pass


def step(name, critical=False):
    def deco(fn):
        t0 = time.time()
        try:
            fn()
            results.append((name, "PASS", ""))
            print(f"PASS  {name}  ({time.time() - t0:.1f}s)", flush=True)
        except Exception as e:  # noqa: BLE001
            msg = re.sub(r"\s+", " ", repr(e))[:500]
            results.append((name, "FAIL", msg))
            print(f"FAIL  {name}: {msg}", flush=True)
            if critical:
                raise Abort(name)
        return fn

    return deco


def ebp(method, path, body=None):
    req = urllib.request.Request(
        f"{BASE}{path}",
        data=json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={"content-type": "application/json", "authorization": f"Bearer {state['api_key']}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except Exception:  # noqa: BLE001
            return e.code, raw.decode(errors="replace")


def payer(method, path, body=None, timeout=120):
    """Call the payer barkd (port 3001 inside the container) with its token."""
    tok = state["payer_token"]
    cmd = (
        f"docker compose exec -T ebp curl -sS -m {timeout} -X {method} "
        f"-H 'Authorization: Bearer {tok}' -H 'content-type: application/json' "
        f"-w '\\n%{{http_code}}' http://127.0.0.1:3001{path}"
    )
    if body is not None:
        cmd += " -d @-"
    rc, out = sh(cmd, inp=json.dumps(body) if body is not None else None, timeout=timeout + 30)
    text, _, code = out.rpartition("\n")
    try:
        return int(code), (json.loads(text) if text.strip() else None)
    except Exception:  # noqa: BLE001
        return int(code) if code.strip().isdigit() else 0, text[:300]


def start_payer():
    sh("docker compose exec -d ebp barkd --datadir /tmp/payer --host 127.0.0.1 --port 3001")
    wait_for(lambda: sh("docker compose exec -T ebp curl -fsS -m 3 http://127.0.0.1:3001/ping")[0] == 0, "payer barkd ping", 60)


def pay(dest, sats, what):
    """Send from the payer; record exactly what barkd answered."""
    code, r = payer("POST", "/api/v1/wallet/send", {"destination": dest, "amount_sat": sats})
    for _ in range(2):
        if code == 200:
            break
        # The payer's own coin bookkeeping can lag right after a send; sync and retry.
        notes.append(f"{what}: first attempt http {code} {str(r)[:160]} -> sync and retry")
        payer("POST", "/api/v1/wallet/sync")
        time.sleep(8)
        code, r = payer("POST", "/api/v1/wallet/send", {"destination": dest, "amount_sat": sats})
    b = balance()
    notes.append(f"{what}: send {sats} -> http {code} {str(r)[:160]} | payer now spendable={b['spendable_sat']} in_round={b['pending_in_round_sat']} arkoor_pending={b['pending_arkoor_send_sat']}")
    assert code == 200, (what, code, r)
    return r


def invoice(order, sats, **kw):
    code, inv = ebp("POST", "/api/v1/invoices", {"order_id": order, "sats": sats, **kw})
    assert code in (200, 201), (code, inv)
    return inv


def inv_state(inv):
    return ebp("GET", f"/api/v1/invoices/{inv['id']}")[1]


def balance():
    code, b = payer("GET", "/api/v1/wallet/balance")
    assert code == 200, (code, b)
    return b


def setup_ebp():
    _, out = sh("docker compose logs --no-color ebp")
    code = wait_for(lambda: (re.findall(r"SETUP CODE: *([A-Z0-9-]+)", sh("docker compose logs --no-color ebp")[1]) or [None])[-1], "setup code", 180, 3)
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        page = browser.new_page()
        page.on("dialog", lambda d: d.accept())
        page.goto(f"{BASE}/setup")
        page.wait_for_selector("#step-code:not(.hidden)")
        page.fill("#code", code)
        page.click("#btn-code")
        page.wait_for_selector("#step-wallet:not(.hidden)")
        page.click("#btn-wallet")
        page.wait_for_selector("#step-backup:not(.hidden)", timeout=180000)
        page.check("#safe-check")
        page.click("#btn-show-words")
        wait_for(lambda: len(page.inner_text("#words").split()) >= 12, "words", 120)
        words = [w for w in page.inner_text("#words").split() if w.isalpha()][-12:]
        page.check("#backup-check")
        page.click("#btn-backup")
        page.wait_for_selector("#quiz:not(.hidden)")
        inputs = page.query_selector_all("#quiz-fields input")
        positions = [int(i.get_attribute("data-pos")) for i in inputs]
        for i, pos in zip(inputs, positions):
            i.fill(words[pos - 1])
        del words
        page.click("#btn-quiz")
        page.wait_for_selector("#step-store:not(.hidden)", timeout=60000)
        page.fill("#store", "Signet Pay Shop")
        page.fill("#admin-pw", ADMIN_PW)
        page.select_option("#payout-kind", "onchain")
        page.fill("#payout", "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx")  # signet placeholder; changed later in the drill
        page.click("#btn-store")
        page.wait_for_selector("#store-secrets:not(.hidden)")
        state["api_key"] = page.inner_text("#api-key").strip()
        browser.close()


def post_issue(title, body):
    """Publish where it can be read while the job is still running (the job
    log and summary are not available until the job ends)."""
    if not os.environ.get("GH_TOKEN"):
        return
    rc, out = sh("gh issue create --title %s --body-file -" % json.dumps(title), inp=body)
    state["issue"] = out.strip().splitlines()[-1] if rc == 0 else None
    print("issue:", state.get("issue") or out[-200:], flush=True)


def comment_issue(body):
    if state.get("issue") and os.environ.get("GH_TOKEN"):
        sh("gh issue comment %s --body-file -" % state["issue"], inp=body)


def summary(text):
    p = os.environ.get("GITHUB_STEP_SUMMARY")
    if p:
        with open(p, "a") as f:
            f.write(text + "\n")


def ebp_admin(method, path, body=None):
    """Call EBP's admin endpoints with the signed-in session (cookie jar + CSRF)."""
    csrf = state.get("csrf", "")
    cmd = (f"curl -sS -m 60 -b jar -c jar -X {method} -H 'content-type: application/json' -H 'x-ebp-csrf: {csrf}' "
           f"-w '\\n%{{http_code}}' {BASE}{path}")
    if body is not None:
        cmd += " -d @-"
    _, out = sh(cmd, inp=json.dumps(body) if body is not None else None, timeout=90)
    text, _, code = out.rpartition("\n")
    try:
        return int(code), (json.loads(text) if text.strip() else None)
    except Exception:  # noqa: BLE001
        return 0, text[:200]


def onchain_total():
    payer("POST", "/api/v1/onchain/sync")
    code, b = payer("GET", "/api/v1/onchain/balance")
    assert code == 200, (code, b)
    return int(b.get("total_sat", 0))


def main():
    @step("setup: EBP's real wizard completes (managed barkd)", critical=True)
    def _():
        setup_ebp()
        assert state["api_key"].startswith("ebp_")
        # The server restarts itself once after the wizard; wait until its
        # wallet is reachable again before creating invoices.
        def wallet_up():
            rc, out = sh(f"curl -fsS {BASE}/api/v1/health")
            try:
                return rc == 0 and json.loads(out)["wallet"]["reachable"]
            except Exception:  # noqa: BLE001
                return False
        wait_for(wallet_up, "EBP's wallet reachable after setup", 180, 3)

    @step("payer: a second barkd starts and creates its own signet wallet", critical=True)
    def _():
        words = None
        if REBUILD:
            # Rebuild the payer's local records from its recovery words (read
            # in memory only, never printed), to drop stale coin bookkeeping.
            rc, w = sh("docker compose exec -T ebp cat /tmp/payer/mnemonic")
            words = " ".join(w.split()) if rc == 0 else None
            assert words and len(words.split()) in (12, 24), "no saved recovery words to rebuild from"
            sh("docker compose exec -T ebp sh -c 'rm -rf /tmp/payer/* /tmp/payer/.[!.]*'")
        start_payer()
        rc, tok = sh("docker compose exec -T ebp barkd --datadir /tmp/payer secret show")
        # barkd prints warnings on the same stream; the token is the one
        # line that is only token characters.
        toks = [l.strip() for l in tok.splitlines() if re.fullmatch(r"[A-Za-z0-9_-]{30,80}", l.strip())]
        state["payer_token"] = toks[-1] if toks else ""
        assert state["payer_token"], "no payer token"
        # The mnemonic file exists only once a wallet was created (a restored one).
        have = (not REBUILD) and sh("docker compose exec -T ebp test -s /tmp/payer/mnemonic")[0] == 0
        if have:
            # Restored from the previous run: the coins you already sent are still here.
            notes.append("Reusing the payer wallet saved by an earlier run.")
            payer("POST", "/api/v1/wallet/sync")
        else:
            body = {"network": "signet"}
            if words:
                body["mnemonic"] = words
            code, out = payer("POST", "/api/v1/wallet/create", body)
            assert code == 200, (code, str(out)[:200])
            if words:
                del words
                for _ in range(6):
                    payer("POST", "/api/v1/wallet/sync")
                    payer("POST", "/api/v1/wallet/sync/mailbox")
                    time.sleep(10)
                notes.append(f"Rebuilt the payer wallet from its saved words: spendable={balance()['spendable_sat']} sats")

    @step("payer: wait for YOU to fund it (watch the job summary / log for the address)", critical=True)
    def _():
        if balance()["spendable_sat"] >= MIN_SATS:
            notes.append(f"Payer already holds {balance()['spendable_sat']} sats from an earlier run; no new funding needed.")
            return
        code, a = payer("POST", "/api/v1/wallet/addresses/next")
        assert code == 200, (code, a)
        ark = a["address"]
        code, o = payer("POST", "/api/v1/onchain/addresses/next")
        onchain = o.get("address") if code == 200 and isinstance(o, dict) else None
        msg = (
            "## SEND SIGNET COINS NOW\n\n"
            f"**Ark address (send from Arké):** `{ark}`\n\n"
            + (f"**On-chain signet address (faucet / on-chain):** `{onchain}`\n\n" if onchain else "")
            + f"Need at least {MIN_SATS} sats spendable. Waiting up to {WAIT_MIN} minutes."
        )
        print("\n" + "=" * 60 + "\n" + msg.replace("**", "") + "\n" + "=" * 60 + "\n", flush=True)
        summary(msg)
        post_issue("signet-pay: send signet coins now", msg)
        # Annotations show up on the check run (readable live through the API).
        print(f"::notice title=SEND SIGNET COINS (Ark)::{ark}", flush=True)
        if onchain:
            print(f"::notice title=SEND SIGNET COINS (on-chain)::{onchain}", flush=True)
        end = time.time() + WAIT_MIN * 60
        boarded = False
        while time.time() < end:
            b = balance()
            if b["spendable_sat"] >= MIN_SATS:
                notes.append(f"Payer funded: spendable {b['spendable_sat']} sats")
                return
            code, ob = payer("GET", "/api/v1/onchain/balance")
            if code == 200 and ob.get("confirmed_sat", 0) >= 10000 and not boarded:
                c2, r2 = payer("POST", "/api/v1/boards/board-all")
                boarded = True
                notes.append(f"Boarded on-chain coins: http {c2}")
            print(f"  waiting... spendable={b['spendable_sat']} pending_board={b['pending_board_sat']} pending_in_round={b['pending_in_round_sat']}", flush=True)
            time.sleep(20)
        raise AssertionError("no funds arrived in time")

    @step("2.2 exact Ark payment: paid within one poll, credited once")
    def _():
        inv = invoice("pay-exact", 2000)
        pay(inv["ark_address"], 2000, "exact")
        got = wait_for(lambda: (s := inv_state(inv))["status"] == "paid" and s, "invoice paid", 180, 3)
        assert got["paid_sats"] == 2000, got
        time.sleep(25)
        assert inv_state(inv)["paid_sats"] == 2000, "must not double-credit"

    @step("2.3 underpay then top up: underpaid, then the top-up completes it")
    def _():
        inv = invoice("pay-under", 3000)
        pay(inv["ark_address"], 1000, "underpay part 1")
        s = wait_for(lambda: (s := inv_state(inv))["paid_sats"] >= 1000 and s, "partial credit", 180, 3)
        notes.append(f"Underpaid invoice shows: status={s['status']} paid_sats={s['paid_sats']} of {s['amount_sats']}")
        assert s["status"] == "underpaid", s["status"]
        pay(inv["ark_address"], 2000, "underpay top-up")
        got = wait_for(lambda: (s := inv_state(inv))["status"] == "paid" and s, "paid after top-up", 180, 3)
        assert got["paid_sats"] == 3000, got

    @step("2.4 overpay: settled as overpaid, full amount recorded")
    def _():
        inv = invoice("pay-over", 1500)
        pay(inv["ark_address"], 2500, "overpay")
        got = wait_for(lambda: (s := inv_state(inv))["status"] in ("paid", "overpaid") and s, "settled", 180, 3)
        assert got["status"] == "overpaid", got
        notes.append(f"Overpaid invoice shows: paid_sats={got['paid_sats']} amount={got['amount_sats']}")
        assert got["paid_sats"] == 2500, got

    @step("2.5 pay after the invoice expired: not credited, appears as an unmatched payment")
    def _():
        inv = invoice("pay-late", 1200, expiry_minutes=1)
        time.sleep(75)
        pay(inv["ark_address"], 1200, "late payment")
        def unmatched():
            code, u = ebp("GET", "/api/v1/unmatched-payments")
            return code == 200 and any(1200 == x.get("amount_sats") for x in (u if isinstance(u, list) else u.get("items", u.get("payments", []))))
        wait_for(unmatched, "unmatched payment listed", 180, 3)
        assert inv_state(inv)["status"] != "paid"

    @step("Lightning: the payer pays EBP's BOLT11 invoice through the ASP gateway")
    def _():
        inv = invoice("pay-ln", 3000)
        assert inv.get("lightning_invoice"), "no lightning invoice on the payment request"
        code, r = payer("POST", "/api/v1/lightning/pay", {"destination": inv["lightning_invoice"]})
        notes.append(f"lightning pay http {code}: {str(r)[:200]}")
        assert code == 200, (code, r)
        got = wait_for(lambda: (s := inv_state(inv))["status"] == "paid" and s, "lightning invoice paid", 240, 4)
        notes.append(f"Lightning invoice credited: paid_sats={got['paid_sats']} of {got['amount_sats']}")

    @step("2.6 server restart mid-payment: nothing lost, nothing doubled")
    def _():
        inv = invoice("pay-restart", 1800)
        pay(inv["ark_address"], 1800, "restart test")
        sh("docker compose restart ebp", timeout=240)
        wait_for(lambda: sh(f"curl -fsS {BASE}/api/v1/health")[0] == 0, "health", 180, 3)
        got = wait_for(lambda: (s := inv_state(inv))["status"] == "paid" and s, "paid after restart", 240, 3)
        time.sleep(25)
        assert inv_state(inv)["paid_sats"] == 1800 == got["paid_sats"]
        start_payer()  # the restart stopped the payer barkd too (same container)

    @step("capture: barkd's real history/movements for the payments above")
    def _():
        rc, out = sh("./scripts/drill_capture.sh")
        assert rc == 0, out[-300:]
        f = sorted(x for x in os.listdir(".") if x.startswith("drill-capture-"))[-1]
        cap = open(f).read()
        notes.append("Real barkd answers after payments:\n" + cap[:30000])

    @step("payout: set the payout address; EBP moves the excess above the till cap there by itself")
    def _():
        code, r = ebp_admin("POST", "/admin/login", {"password": ADMIN_PW})
        assert code == 200, ("admin login", code)
        state["csrf"] = (r or {}).get("csrf", "")
        code, a = payer("POST", "/api/v1/wallet/addresses/next")
        assert code == 200, (code, a)
        dest = a["address"]
        before = balance()["spendable_sat"]
        code, r = ebp_admin("POST", "/admin/payout", {"address": dest, "admin_password": ADMIN_PW})
        notes.append(f"set payout address -> http {code}")
        if code != 200:
            _, jar = sh("awk '{print $1, $2, $4, $6}' jar")
            notes.append("cookie jar (domain, subdomains, secure, name):\n" + jar + f"\nresponse: {str(r)[:200]}")
        assert code == 200, (code, r)
        def grew():
            payer("POST", "/api/v1/wallet/sync")
            payer("POST", "/api/v1/wallet/sync/mailbox")
            return balance()["spendable_sat"] > before

        try:
            wait_for(grew, "the automatic transfer to arrive in the payout wallet", 360, 15)
        finally:
            _, lg = sh("docker compose logs --no-color --tail 300 ebp")
            hits = [l[-240:] for l in lg.splitlines() if "sweep" in l.lower() or "reserve" in l.lower() or "settle" in l.lower()][-10:]
            notes.append("server log (sweep lines):\n" + "\n".join(hits))
        notes.append(f"payer spendable before={before} after={balance()['spendable_sat']}")
        assert balance()["spendable_sat"] > before, "payout wallet did not receive the sweep"

    @step("on-chain sweep: payout set to an on-chain address; EBP withdraws there once fees are low")
    def _():
        code, o = payer("POST", "/api/v1/onchain/addresses/next")
        assert code == 200 and o.get("address"), (code, o)
        dest = o["address"]
        notes.append(f"on-chain payout address is a {dest[:3]}… address")
        code, r = ebp_admin("POST", "/admin/payout", {"address": dest, "admin_password": ADMIN_PW})
        assert code == 200, ("switch payout to on-chain", code, r)
        # What the network charges right now (EBP's own wallet answers).
        rc, tok = sh("docker compose exec -T ebp barkd --datadir /data/barkd secret show")
        etok = [l.strip() for l in tok.splitlines() if re.fullmatch(r"[A-Za-z0-9_-]{30,80}", l.strip())][-1]
        _, fees = sh(f"docker compose exec -T ebp curl -sS -m 30 -H 'Authorization: Bearer {etok}' http://127.0.0.1:3000/api/v1/fees/onchain")
        notes.append(f"network fee rates (sat/vB): {fees.strip()[:200]}")
        before = onchain_total()
        # Give the till something over the on-chain cap (set small for this drill).
        inv = invoice("pay-onchain-sweep", 6000)
        pay(inv["ark_address"], 6000, "funds for the on-chain sweep")
        wait_for(lambda: inv_state(inv)["status"] in ("paid", "overpaid"), "invoice paid", 180, 3)
        def arrived():
            now = onchain_total()
            return now > before
        try:
            wait_for(arrived, "the on-chain withdrawal to show up in the payout wallet", 900, 20)
        finally:
            _, lg = sh("docker compose logs --no-color --tail 400 ebp")
            hits = [l[-240:] for l in lg.splitlines() if "sweep" in l.lower() or "fee" in l.lower()][-10:]
            notes.append("server log (on-chain sweep lines):\n" + "\n".join(hits))
        notes.append(f"payout wallet on-chain total before={before} after={onchain_total()}")

    @step("manual sweep button: the whole till moves to the payout address on request")
    def _():
        # Switch the payout back to an Ark address (free to test), put LESS than
        # the Ark cap in the till so the automatic sweep stays quiet, then press
        # the button.
        code, a = payer("POST", "/api/v1/wallet/addresses/next")
        assert code == 200, (code, a)
        code, r = ebp_admin("POST", "/admin/payout", {"address": a["address"], "admin_password": ADMIN_PW})
        assert code == 200, ("switch payout back to Ark", code, r)
        # Empty the till first (also the first, unmeasured use of the button), so
        # the 1500 below is the only thing in it and sits under the Ark cap.
        ebp_admin("POST", "/admin/sweep", {})
        time.sleep(60)
        payer("POST", "/api/v1/wallet/sync"); payer("POST", "/api/v1/wallet/sync/mailbox")
        before = balance()["spendable_sat"]
        inv = invoice("pay-manual-sweep", 1500)
        pay(inv["ark_address"], 1500, "funds for the manual sweep")
        wait_for(lambda: inv_state(inv)["status"] in ("paid", "overpaid"), "invoice paid", 180, 3)
        mid = balance()["spendable_sat"]
        time.sleep(70)  # two settlement ticks: below the cap, nothing may move by itself
        payer("POST", "/api/v1/wallet/sync"); payer("POST", "/api/v1/wallet/sync/mailbox")
        quiet = balance()["spendable_sat"]
        notes.append(f"before pressing the button: payer spendable {mid} -> {quiet} (must not grow by itself)")
        assert quiet <= mid + 5, "an automatic sweep moved money that was under the cap"
        code, r = ebp_admin("POST", "/admin/sweep", {})
        notes.append(f"manual sweep -> http {code} {str(r)[:200]}")
        assert code == 200 and (r or {}).get("swept_sats"), (code, r)
        def arrived():
            payer("POST", "/api/v1/wallet/sync"); payer("POST", "/api/v1/wallet/sync/mailbox")
            return balance()["spendable_sat"] > quiet
        wait_for(arrived, "the manual sweep to arrive", 240, 10)
        notes.append(f"payer spendable after the manual sweep: {balance()['spendable_sat']}")

    @step("keep the coins: EBP's wallet goes back to the payer wallet (or RETURN_TO)")
    def _():
        dest = RETURN_TO
        if not dest:
            code, a = payer("POST", "/api/v1/wallet/addresses/next")
            assert code == 200, (code, a)
            dest = a["address"]
            notes.append("No RETURN_TO: EBP's coins move back into the saved payer wallet; the workflow keeps that wallet for the next run.")

        def sweep(call, bal, what):
            amt = bal()
            for fee_guess in (0, 300, 1000, 3000):
                a = amt - fee_guess
                if a < 1000:
                    notes.append(f"{what}: nothing worth returning ({amt} sats)")
                    return True
                code, r = call(a)
                notes.append(f"{what}: return {a} sats -> http {code} {str(r)[:140]}")
                if code == 200:
                    return True
            return False

        ok1 = True
        if RETURN_TO:
            ok1 = sweep(
                lambda a: payer("POST", "/api/v1/wallet/send", {"destination": RETURN_TO, "amount_sat": a}),
                lambda: balance()["spendable_sat"],
                "payer wallet",
            )
        # EBP's own wallet received the test payments; send those back too.
        rc, tok = sh("docker compose exec -T ebp barkd --datadir /data/barkd secret show")
        etok = tok.strip().splitlines()[-1].strip()

        def ebp_barkd(method, path, body=None):
            cmd = (
                f"docker compose exec -T ebp curl -sS -m 120 -X {method} -H 'Authorization: Bearer {etok}' "
                f"-H 'content-type: application/json' -w '\\n%{{http_code}}' http://127.0.0.1:3000{path}"
            )
            if body is not None:
                cmd += " -d @-"
            _, out = sh(cmd, inp=json.dumps(body) if body is not None else None, timeout=180)
            text, _, code = out.rpartition("\n")
            try:
                return int(code), (json.loads(text) if text.strip() else None)
            except Exception:  # noqa: BLE001
                return 0, text[:200]

        # Coins mid-refresh are locked and cannot be sent. Wait for EBP's wallet
        # to settle first, or they would be stranded on this (destroyed) machine.
        def settled():
            code, b = ebp_barkd("GET", "/api/v1/wallet/balance")
            if code != 200 or not isinstance(b, dict):
                return False
            return b.get("pending_in_round_sat", 0) == 0 and b.get("pending_board_sat", 0) == 0
        try:
            wait_for(settled, "EBP's wallet to finish any refresh round", 600, 15)
        except Exception as e:  # noqa: BLE001
            notes.append(f"WARNING: EBP's wallet still mid-refresh after 10 minutes: {e}")

        ok2 = sweep(
            lambda a: ebp_barkd("POST", "/api/v1/wallet/send", {"destination": dest, "amount_sat": a}),
            lambda: ebp_barkd("GET", "/api/v1/wallet/balance")[1]["spendable_sat"],
            "EBP wallet",
        )
        assert ok1 and ok2, "could not return all funds"
        # Let the payer wallet see what just arrived before the workflow saves it.
        for _ in range(6):
            time.sleep(10)
            payer("POST", "/api/v1/wallet/sync")
            payer("POST", "/api/v1/wallet/sync/mailbox")
        notes.append(f"Payer wallet saved with spendable={balance()['spendable_sat']} sats")


try:
    main()
except Abort as a:
    print(f"ABORTED at: {a}")
finally:
    lines = ["# Signet payment drill report", "", "| Step | Result | Notes |", "|---|---|---|"]
    for n, r, m in results:
        lines.append(f"| {n} | {r} | {m.replace('|', '/')} |")
    lines += ["", "## Observations", ""] + [f"```\n{n}\n```" for n in notes]
    open("pay-report.md", "w").write("\n".join(lines) + "\n")
    print("\n".join(lines))
    summary("\n".join(lines))
    comment_issue("\n".join(lines)[:60000])
    sys.exit(0 if results and all(r == "PASS" for _, r, _ in results) else 1)
