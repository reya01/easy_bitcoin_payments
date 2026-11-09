#!/usr/bin/env python3
"""A fake barkd for end-to-end tests.

Speaks the REST API EBP uses, with state (wallet, VTXOs, history, on-chain
transactions) and a control channel for tests. Every request body and every
response is validated against the pinned OpenAPI spec (docs/barkd-openapi.json),
so the fake cannot drift from the real shapes; any violation is recorded and
the test run fails on it (GET /__ctl/violations).

Not the real daemon: it cannot prove that real barkd behaves this way, only
that EBP behaves correctly against responses that match the spec.

Usage: fake_barkd.py <port> <token>
"""
import hashlib
import json
import os
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import jsonschema

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SPEC = json.load(open(os.path.join(ROOT, "docs", "barkd-openapi.json")))
COMPONENTS = SPEC["components"]

WORDS = ("abandon ability able about above absent absorb abstract absurd abuse access accident").split()

LOCK = threading.Lock()
S = {}
VIOLATIONS = []


def reset():
    S.clear()
    S.update(
        created=False,
        tip=800,
        connected=True,
        needs_refresh=0,
        onchain_confirmed=200_000,
        down=False,
        hang=False,
        garbage="",  # "": normal. Otherwise what a broken daemon answers with (see GARBAGE)
        n_addr=0,
        n_move=0,
        n_vtxo=0,
        vtxos=[],
        moves=[],
        txs=[],
        log=[],
    )


reset()

# ---------------------------------------------------------------- bech32 ----
CH = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def _polymod(values):
    gen = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    chk = 1
    for v in values:
        b = chk >> 25
        chk = (chk & 0x1FFFFFF) << 5 ^ v
        for i in range(5):
            chk ^= gen[i] if (b >> i) & 1 else 0
    return chk


def _hrp_expand(h):
    return [ord(x) >> 5 for x in h] + [0] + [ord(x) & 31 for x in h]


def _convert(data, frm, to):
    acc, bits, out, maxv = 0, 0, [], (1 << to) - 1
    for v in data:
        acc = (acc << frm) | v
        bits += frm
        while bits >= to:
            bits -= to
            out.append((acc >> bits) & maxv)
    if bits:
        out.append((acc << (to - bits)) & maxv)
    return out


def segwit_address(hrp, program20):
    data = [0] + _convert(program20, 8, 5)
    values = _hrp_expand(hrp) + data
    mod = _polymod(values + [0] * 6) ^ 1
    checksum = [(mod >> 5 * (5 - i)) & 31 for i in range(6)]
    return hrp + "1" + "".join(CH[d] for d in data + checksum)


def script_for(address):
    sep = address.rfind("1")
    data = [CH.index(c) for c in address[sep + 1 : -6]]
    program = bytes(_convert(data[1:], 5, 8)[:20])
    return bytes([0x00, 0x14]) + program


def raw_tx(address, sats):
    out_script = script_for(address)
    tx = bytes.fromhex("01000000" "01")
    tx += hashlib.sha256(address.encode() + str(sats).encode()).digest()
    tx += bytes.fromhex("00000000" "00" "ffffffff")
    tx += bytes([1]) + sats.to_bytes(8, "little") + bytes([len(out_script)]) + out_script
    tx += bytes.fromhex("00000000")
    return tx.hex()


def txid_for(label):
    return hashlib.sha256(label.encode()).hexdigest()


# -------------------------------------------------------------- validation --
def _check(instance, schema, what):
    doc = {**schema, "components": COMPONENTS}
    try:
        jsonschema.validate(instance, doc)
    except jsonschema.ValidationError as e:
        VIOLATIONS.append(f"{what}: {e.message} at {list(e.absolute_path)}")


def op_for(method, path):
    return SPEC["paths"].get("/api/v1" + path, {}).get(method.lower())


def iso(ts):
    import datetime

    return datetime.datetime.fromtimestamp(ts, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ------------------------------------------------------------------ logic --
def new_move(kind_name, kind, amount, received_on=None, sent_to=None, outs=None, meta=None):
    S["n_move"] += 1
    now = iso(1_762_000_000 + S["n_move"])
    return {
        "id": S["n_move"],
        "status": "successful",
        "subsystem": {"name": kind_name, "kind": kind},
        "intended_balance_sat": amount,
        "effective_balance_sat": amount,
        "offchain_fee_sat": 0,
        "sent_to": sent_to or [],
        "received_on": received_on or [],
        "input_vtxos": [],
        "output_vtxos": outs or [],
        "exited_vtxos": [],
        "metadata": meta,
        "time": {"created_at": now, "updated_at": now, "completed_at": now},
    }


def add_vtxo(sats):
    S["n_vtxo"] += 1
    vid = f"{txid_for('vtxo%d' % S['n_vtxo'])}:0"
    S["vtxos"].append(
        {
            "id": vid,
            "amount_sat": sats,
            "chain_anchor": f"{txid_for('anchor%d' % S['n_vtxo'])}:0",
            "exit_delta": 12,
            "expiry_height": S["tip"] + 4000,
            "policy_type": "pubkey",
            "server_pubkey": "02" + "11" * 32,
            "user_pubkey": "03" + "22" * 32,
            "state": {"type": "spendable"},
        }
    )
    return vid


def spendable_total():
    return sum(v["amount_sat"] for v in S["vtxos"] if v["state"]["type"] == "spendable")


def handle(method, path, query, body):
    """Return (status, json) for a data endpoint, or None if unknown."""
    if path == "/wallet/create" and method == "POST":
        S["created"] = True
        return 200, {"fingerprint": "deadbeef"}
    if path == "/wallet/mnemonic":
        if not S["created"]:
            return 404, {"message": "no wallet", "resources": []}
        return 200, {"mnemonic": " ".join(WORDS)}
    if path == "/wallet/connected":
        return 200, {"connected": S["connected"]}
    if path == "/wallet/ark-info":
        # The shape of the real signet server's answer (8 Nov 2026 drill):
        # one-day coins, refresh free below 288 blocks.
        return 200, {
            "network": "signet",
            "vtxo_lifetime": 144,
            "vtxo_expiry_delta": 144,
            "fees": {
                "refresh": {
                    "base_fee_sat": 0,
                    "ppm_expiry_table": [
                        {"expiry_blocks_threshold": 0, "ppm": 0},
                        {"expiry_blocks_threshold": 288, "ppm": 2000},
                        {"expiry_blocks_threshold": 1008, "ppm": 4000},
                        {"expiry_blocks_threshold": 2016, "ppm": 5000},
                    ],
                }
            },
        }
    if path == "/bitcoin/tip":
        return 200, {"tip_height": S["tip"]}
    if path == "/wallet/addresses/next":
        S["n_addr"] += 1
        return 200, {"address": "tark1q" + hashlib.sha256(b"ark%d" % S["n_addr"]).hexdigest()[:40]}
    if path == "/onchain/addresses/next":
        S["n_addr"] += 1
        prog = hashlib.sha256(b"btc%d" % S["n_addr"]).digest()[:20]
        return 200, {"address": segwit_address("tb", prog)}
    if path == "/wallet/balance":
        z = 0
        return 200, {
            "spendable_sat": spendable_total(),
            "needs_refresh_sat": S["needs_refresh"],
            "claimable_lightning_receive_sat": z,
            "pending_arkoor_send_sat": z,
            "pending_board_sat": z,
            "pending_exit_sat": z,
            "pending_in_round_sat": z,
            "pending_lightning_send_sat": z,
            "pending_offboard_sat": z,
        }
    if path == "/onchain/balance":
        c = S["onchain_confirmed"]
        return 200, {
            "confirmed_sat": c,
            "total_sat": c,
            "trusted_spendable_sat": c,
            "immature_sat": 0,
            "trusted_pending_sat": 0,
            "untrusted_pending_sat": 0,
        }
    if path == "/wallet/vtxos":
        return 200, list(S["vtxos"])
    if path == "/history":
        return 200, list(reversed(S["moves"]))
    if path == "/onchain/transactions":
        return 200, list(S["txs"])
    if path in ("/wallet/refresh/all", "/wallet/refresh/vtxos"):
        return 200, {
            "id": 1,
            "state": "pending",
            "status": {"status": "pending"},
            "funding_txid": None,
            "funding_tx_hex": None,
            "scheduled_height": None,
            "unlock_hash": None,
            "participation": {"inputs": [], "outputs": []},
        }
    if path == "/wallet/send":
        amt = body.get("amount_sat") or 0
        S["moves"].append(
            new_move(
                "bark.arkoor",
                "send",
                -amt,
                sent_to=[{"destination": {"type": "ark", "value": body["destination"]}, "amount_sat": amt}],
            )
        )
        S["vtxos"] = [v for v in S["vtxos"] if v["amount_sat"] != amt] or S["vtxos"]
        return 200, {"message": "sent", "payment_hash": None}
    if path in ("/wallet/offboard/all", "/wallet/offboard/vtxos"):
        return 200, {"movement_id": 1, "offboard_txid": txid_for("offboard")}
    if path == "/lightning/receives/invoice":
        return 200, {"invoice": "lntbs%dn1fake%s" % (body["amount_sat"], "q" * 20)}
    if path == "/fees/lightning/receive":
        a = int(query.get("amount_sat", ["0"])[0])
        return 200, {"gross_amount_sat": a, "fee_sat": 3, "net_amount_sat": a - 3, "vtxos_spent": []}
    return None


def control(body):
    act = body.get("action")
    if act == "reset":
        reset()
        VIOLATIONS.clear()
    elif act == "set":
        S[body["key"]] = body["value"]
    elif act == "pay_ark":
        vid = add_vtxo(body["sats"])
        S["moves"].append(
            new_move(
                "bark.arkoor",
                "receive",
                body["sats"],
                received_on=[{"destination": {"type": "ark", "value": body["address"]}, "amount_sat": body["sats"]}],
                outs=[vid],
            )
        )
    elif act == "pay_onchain":
        txid = body.get("txid") or txid_for("tx%d" % len(S["txs"]))
        S["txs"] = [t for t in S["txs"] if t["txid"] != body.get("replaces")]
        S["txs"].append(
            {
                "txid": txid,
                "tx": raw_tx(body["address"], body["sats"]),
                "balance_change_sat": body["sats"],
                "is_cpfp": False,
                "confirmation": None,
            }
        )
    elif act == "mine":
        S["tip"] += body.get("blocks", 1)
        for t in S["txs"]:
            if t["confirmation"] is None and body.get("confirm", True):
                t["confirmation"] = {"height": S["tip"], "hash": txid_for("blk%d" % S["tip"])}
    return {"ok": True, "tip": S["tip"]}


# What a broken or misconfigured daemon (or a proxy in front of it) might send
# back instead of the real answer: (status, content type, body).
GARBAGE = {
    "html": (200, "text/html", b"<html><body><h1>502 Bad Gateway</h1></body></html>"),
    "truncated": (200, "application/json", b'{"balance": {"spendable_sat": 12'),
    "empty": (200, "application/json", b""),
    "null": (200, "application/json", b"null"),
    "array": (200, "application/json", b"[1,2,3]"),
    "string": (200, "application/json", b'"hello"'),
    "wrong_types": (200, "application/json", b'{"spendable_sat":"lots","address":42,"history":{},"moves":"x"}'),
    "huge": (200, "application/json", b'{"x":"' + b"a" * 3_000_000 + b'"}'),
    "binary": (200, "application/octet-stream", bytes(range(256)) * 40),
    "error_html": (500, "text/html", b"<html>Internal Server Error</html>"),
    "slow_error": (503, "application/json", b'{"message":"try later"}'),
}


class H(BaseHTTPRequestHandler):
    token = ""

    def log_message(self, *a):
        pass

    def _send(self, code, obj):
        raw = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _send_garbage(self, kind):
        code, ctype, raw = GARBAGE[kind]
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _body(self):
        n = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(n) if n else b""
        return json.loads(raw) if raw.strip() else {}

    def _serve(self, method):
        u = urlparse(self.path)
        query = parse_qs(u.query)
        try:
            body = self._body()
        except Exception:
            body = {}
        if u.path.startswith("/api/v1/"):
            # A hung daemon: accept the request, never answer (until released).
            waited = 0.0
            while S.get("hang") and waited < 120:
                time.sleep(0.1)
                waited += 0.1
        with LOCK:
            if u.path == "/__ctl":
                return self._send(200, control(body))
            if u.path == "/__ctl/violations":
                return self._send(200, VIOLATIONS)
            if u.path == "/__ctl/state":
                return self._send(200, {k: v for k, v in S.items() if k != "log"})
            if u.path == "/ping":
                return self._send(200, {})
            if not u.path.startswith("/api/v1/"):
                return self._send(404, {"message": "not found"})
            path = u.path[len("/api/v1") :]
            S["log"].append(f"{method} {path}")
            if S["down"]:
                return self._send(500, {"message": "fake barkd is down"})
            if S.get("garbage"):
                return self._send_garbage(S["garbage"])
            if path != "/wallet/create":
                auth = self.headers.get("authorization", "")
                if auth != f"Bearer {self.token}":
                    return self._send(401, {"message": "unauthorized"})
            op = op_for(method, path)
            if op is None or op.get("deprecated"):
                VIOLATIONS.append(f"{method} {path}: not a current endpoint in the pinned spec")
                return self._send(404, {"message": "not found"})
            rb = (op.get("requestBody") or {}).get("content", {}).get("application/json", {}).get("schema")
            if rb is not None:
                _check(body, rb, f"request {method} {path}")
            for p in op.get("parameters", []):
                if p.get("required") and p["in"] == "query" and p["name"] not in query:
                    VIOLATIONS.append(f"{method} {path}: missing required query {p['name']}")
            if not S["created"] and path not in ("/wallet/create", "/wallet/mnemonic"):
                # A barkd with no wallet yet answers data endpoints with an
                # error; EBP's setup status relies on that to know whether the
                # wizard still has to create the wallet.
                return self._send(500, {"message": "no wallet has been created yet"})
            res = handle(method, path, query, body)
            if res is None:
                VIOLATIONS.append(f"{method} {path}: fake has no handler")
                return self._send(501, {"message": "not implemented in fake"})
            code, obj = res
            rsp = (op.get("responses") or {}).get(str(code), {}).get("content", {}).get("application/json", {}).get("schema")
            if rsp is not None:
                _check(obj, rsp, f"response {method} {path} {code}")
            else:
                VIOLATIONS.append(f"{method} {path}: spec defines no {code} response body")
            return self._send(code, obj)

    def do_GET(self):
        self._serve("GET")

    def do_POST(self):
        self._serve("POST")


if __name__ == "__main__":
    port, token = int(sys.argv[1]), sys.argv[2]
    H.token = token
    ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
