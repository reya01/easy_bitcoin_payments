#!/usr/bin/env python3
"""Mistakes at the command line and in settings must give a plain message and a
failure code, never a crash dump, a Rust debug dump or a hang.

Run: python3 tests/e2e/startup_errors.py target/release/ebp
"""
import os, subprocess, sys, tempfile

EBP = os.path.abspath(sys.argv[1])
tmp = tempfile.mkdtemp(prefix="ebp-startup-")
bad_toml = os.path.join(tmp, "bad.toml")
open(bad_toml, "w").write("this is [not valid toml\n")

BASE_ENV = {"PATH": os.environ.get("PATH", ""), "HOME": tmp, "EBP_UPDATE_CHECK": "false",
            "EBP_DATABASE_PATH": os.path.join(tmp, "db", "ebp.sqlite3")}

# name, extra env, args, stdin, must fail, text the message must contain (any of)
CASES = [
    ("unknown command", {}, ["frobnicate"], "", True, ["unknown command"]),
    ("bind address that is not an address", {"EBP_BIND_ADDR": "notanaddress"}, ["serve"], "", True, ["EBP_BIND_ADDR"]),
    ("mode that does not exist", {"EBP_MODE": "banana"}, ["serve"], "", True, ["banana", "EBP_MODE"]),
    ("true/false setting with a wrong word", {"EBP_ONCHAIN_RAIL_ENABLED": "maybe"}, ["serve"], "", True, ["EBP_ONCHAIN_RAIL_ENABLED"]),
    ("number setting with letters", {"EBP_DEFAULT_EXPIRY_MINUTES": "soon"}, ["serve"], "", True, ["EBP_DEFAULT_EXPIRY_MINUTES"]),
    ("interval of zero", {"EBP_WATCHER_INTERVAL_SECS": "0"}, ["serve"], "", True, ["EBP_WATCHER_INTERVAL_SECS"]),
    ("payout address that is not an address", {"EBP_SETTLEMENT_DESTINATION": "notanaddress"}, ["serve"], "", True, []),
    ("config file that is missing", {"EBP_CONFIG": os.path.join(tmp, "nope.toml")}, ["serve"], "", True, ["config file"]),
    ("config file that is not valid", {"EBP_CONFIG": bad_toml}, ["serve"], "", True, ["config file"]),
    ("encryption key that is garbage", {"EBP_KEY_ENCRYPTION_KEY": "zz"}, ["serve"], "", True, []),
    ("database in a place that cannot exist", {"EBP_DATABASE_PATH": "/proc/nope/ebp.sqlite3"}, ["serve"], "", True, ["database"]),
    ("backup verify on a missing file", {}, ["backup", "verify", os.path.join(tmp, "missing.ebpbackup")], "pw\n", True, []),
    ("backup verify on a file that is not a backup", {}, ["backup", "verify", bad_toml], "pw\n", True, []),
    ("backup restore into a folder that exists", {}, ["backup", "restore", bad_toml, tmp], "pw\n", True, []),
    ("store create without a name", {}, ["store", "create"], "", True, []),
    ("store rotate-key for a store that is not there", {}, ["store", "rotate-key", "--store", "sto_nope"], "", True, []),
    ("admin set-password with a too-short password", {}, ["admin", "set-password"], "short\n", True, []),
    ("admin set-password with nothing typed", {}, ["admin", "set-password"], "", True, []),
    ("admin with a word that is not a command", {}, ["admin", "dance"], "", True, ["usage"]),
]

failures = 0
for name, extra, args, stdin, must_fail, mention in CASES:
    env = dict(BASE_ENV)
    env.update(extra)
    try:
        r = subprocess.run([EBP] + args, input=stdin, capture_output=True, text=True, timeout=20, env=env, cwd=tmp)
    except subprocess.TimeoutExpired:
        print(f"FAIL  {name}: did not finish in 20 s (hung)")
        failures += 1
        continue
    out = (r.stdout or "") + (r.stderr or "")
    problems = []
    if must_fail and r.returncode == 0:
        problems.append("succeeded but should have refused")
    if "panicked" in out or "RUST_BACKTRACE" in out or r.returncode in (101, -6, -11):
        problems.append("crashed (panic)")
    if "Invalid {" in out or "Error: \"" in out or "Os {" in out or "Custom {" in out:
        problems.append("shows Rust debug text instead of a plain message")
    if mention and not any(m.lower() in out.lower() for m in mention):
        problems.append(f"message does not mention {mention}: {out.strip()[:140]!r}")
    if not out.strip():
        problems.append("fails silently (no message at all)")
    print(("FAIL  " if problems else "PASS  ") + name + (("  -> " + "; ".join(problems) + f"   [exit {r.returncode}, output: {out.strip()[:200]!r}]") if problems else ""))
    failures += bool(problems)
sys.exit(1 if failures else 0)
