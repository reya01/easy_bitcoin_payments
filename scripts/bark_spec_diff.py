#!/usr/bin/env python3
"""Summarise how one barkd OpenAPI spec differs from another, in plain words.

Usage: scripts/bark_spec_diff.py OLD_SPEC NEW_SPEC
Prints paths added or removed, operations or schemas changed, and exits 0 either way:
the caller decides what counts as a problem (the contract and drift checks do that).
"""
import json
import sys

old = json.load(open(sys.argv[1]))
new = json.load(open(sys.argv[2]))


def part(d, key):
    return d.get(key, {}) if key != "schemas" else d.get("components", {}).get("schemas", {})


op, np_ = part(old, "paths"), part(new, "paths")
os_, ns = part(old, "schemas"), part(new, "schemas")
print(f"spec versions: {old.get('info', {}).get('version')} -> {new.get('info', {}).get('version')}")
print(f"paths: {len(op)} -> {len(np_)}")
lines = []
for p in sorted(set(np_) - set(op)):
    lines.append(f"  added path    {p}")
for p in sorted(set(op) - set(np_)):
    lines.append(f"  REMOVED path  {p}")
for p in sorted(set(op) & set(np_)):
    if op[p] != np_[p]:
        lines.append(f"  changed path  {p}")
for s in sorted(set(ns) - set(os_)):
    lines.append(f"  added schema  {s}")
for s in sorted(set(os_) - set(ns)):
    lines.append(f"  REMOVED schema {s}")
for s in sorted(set(os_) & set(ns)):
    if os_[s] != ns[s]:
        lines.append(f"  changed schema {s}")
print("\n".join(lines) if lines else "  no differences besides the version number")
