#!/usr/bin/env python3
"""Field-level barkd contract check (complements check_barkd_drift.sh, which
only checks paths and HTTP verbs).

For every entry in docs/barkd-contract.json, against docs/barkd-openapi.json:
  * the path + method exist and are not deprecated;
  * every field we SEND exists in the request schema, and every field the
    spec marks REQUIRED is among the fields we send;
  * every query parameter we send exists, and required ones are covered;
  * every field we READ exists in the 200 response schema (dotted paths walk
    through objects, arrays and oneOf/anyOf/allOf).
Exit 1 on any mismatch. Usage: scripts/check_barkd_contract.py [spec] [table]
"""
import json
import sys

spec_path = sys.argv[1] if len(sys.argv) > 1 else "docs/barkd-openapi.json"
table_path = sys.argv[2] if len(sys.argv) > 2 else "docs/barkd-contract.json"
spec = json.load(open(spec_path))
table = json.load(open(table_path))
schemas = spec.get("components", {}).get("schemas", {})


def resolve(node):
    while isinstance(node, dict) and "$ref" in node:
        node = schemas[node["$ref"].split("/")[-1]]
    return node


def variants(node, depth=0):
    """Flatten oneOf/anyOf/allOf into a list of concrete object-ish schemas."""
    node = resolve(node)
    if not isinstance(node, dict) or depth > 8:
        return []
    out = [node]
    for key in ("oneOf", "anyOf", "allOf"):
        for sub in node.get(key, []):
            out += variants(sub, depth + 1)
    return out


def props(node):
    merged, required = {}, set()
    for v in variants(node):
        merged.update(v.get("properties", {}))
        required |= set(v.get("required", []))
    return merged, required


def has_path(node, dotted):
    """True if the dotted path exists under `node` (arrays are transparent)."""
    head, _, rest = dotted.partition(".")
    for v in variants(node):
        if v.get("type") == "array" or "items" in v:
            if has_path(v.get("items", {}), dotted):
                return True
    p, _ = props(node)
    if head not in p:
        return False
    return has_path(p[head], rest) if rest else True


def schema_of(content):
    return (content or {}).get("application/json", {}).get("schema")


errors, checked = [], 0
for e in table["endpoints"]:
    method, path = e["method"].lower(), "/api/v1" + e["path"]
    label = f'{e["method"]} {e["path"]}'
    op = spec["paths"].get(path, {}).get(method)
    if not op:
        errors.append(f"{label}: not in the pinned spec")
        continue
    checked += 1
    if op.get("deprecated"):
        errors.append(f"{label}: DEPRECATED in the pinned spec")
    if "sends" in e:
        body = schema_of((op.get("requestBody") or {}).get("content"))
        sp, req = props(body) if body else ({}, set())
        for f in e["sends"]:
            if f not in sp:
                errors.append(f"{label}: we send '{f}' but the request schema has no such field")
        for f in sorted(req - set(e["sends"])):
            errors.append(f"{label}: spec REQUIRES '{f}' but we never send it")
        if body is None and e["sends"]:
            errors.append(f"{label}: we send a body but the spec defines none")
    params = [resolve(p) for p in op.get("parameters", [])]
    qspec = {p["name"]: p for p in params if p.get("in") == "query"}
    for q in e.get("query", []):
        if q not in qspec:
            errors.append(f"{label}: query '{q}' is not a spec parameter")
    for name, p in qspec.items():
        if p.get("required") and name not in e.get("query", []):
            errors.append(f"{label}: spec REQUIRES query '{name}' but we never send it")
    resp = schema_of(op.get("responses", {}).get("200", {}).get("content"))
    for f in e.get("reads", []):
        if resp is None or not has_path(resp, f):
            errors.append(f"{label}: we read '{f}' but the 200 response schema has no such field")

if checked == 0:
    errors.append("no endpoints checked: the table is empty or the spec paths changed shape")
for m in errors:
    print("FAIL ", m)
print(f"barkd contract: {checked} endpoint(s) checked, {len(errors)} problem(s)")
sys.exit(1 if errors else 0)
