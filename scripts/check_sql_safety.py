#!/usr/bin/env python3
"""Fail if Rust code builds SQL text by splicing values into it.

Every value that comes from a customer, a shop or the owner must reach the
database as a bound parameter (`?1`, `?2`, ...), never pasted into the SQL
string. This check looks for the usual ways of pasting: `format!(` (or
`push_str(` / `concat!(` / `+ "`) around SQL words with `{...}` in the text.

A few places paste a fixed constant or a literal table name on purpose. They
are listed in ALLOWED below, each with the reason. Adding to the list needs a
reviewer to agree that the pasted part can never come from outside.

Run from the repository root:  python3 scripts/check_sql_safety.py
"""
import re
import sys
from pathlib import Path

SQL_WORDS = re.compile(
    r"\b(SELECT|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP|PRAGMA|ATTACH|WHERE|ORDER BY|LIMIT|FROM)\b"
)
HAS_PLACEHOLDER = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*(:[^}]*)?\}|\{\}")
# `?{}` pastes the NUMBER of a bound parameter (`?3`), which is fine: the value
# itself still travels as a parameter.
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')
PARAM_NUMBER = re.compile(r"\?\{[^}]*\}")
# `format!(` plus the next lines of the same statement (a few lines is enough).
BUILDERS = re.compile(r"(format!\(|push_str\(|concat!\(|\+\s*&?format!\()")
WINDOW = 8  # at most this many lines of one statement are looked at

# (file, exact stripped source line, reason)
ALLOWED = [
    ("src/invoice.rs", 'let mut stmt = conn.prepare(&format!(',
     "pastes ONCHAIN_HOLD_SQL, a fixed constant in the same file"),
    ("src/invoice.rs", '&format!("SELECT {ONCHAIN_HOLD_SQL} FROM invoices WHERE id = ?2"),',
     "pastes ONCHAIN_HOLD_SQL, a fixed constant in the same file"),
    ("src/backup.rs", 'conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))',
     "table is one of the literal names \"stores\" / \"invoices\" passed by the two call sites"),
    ("src/backup.rs", '.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))',
     "test only, literal table names"),
    ("src/webhook_outbox.rs", '&format!("SELECT COUNT(*) FROM webhook_deliveries WHERE {where_sql}"),',
     "test only, literal conditions"),
]


def allowed(path: str, line: str) -> bool:
    return any(path == f and line.strip() == l for f, l, _ in ALLOWED)


def main() -> int:
    bad = []
    for path in sorted(Path("src").rglob("*.rs")):
        rel = path.as_posix()
        lines = path.read_text(encoding="utf-8").splitlines()
        for i, line in enumerate(lines):
            if not BUILDERS.search(line):
                continue
            # Join the rest of this statement and take the first string
            # literal after the builder: that is the text being built.
            stmt = []
            for text in lines[i : i + WINDOW]:
                stmt.append(text)
                if text.rstrip().endswith(";"):  # end of the statement
                    break
            joined = " ".join(stmt)
            start = BUILDERS.search(joined).end()
            lit = LITERAL.search(joined, start)
            if not lit:
                continue
            text = PARAM_NUMBER.sub("?N", lit.group(1))
            if SQL_WORDS.search(text) and HAS_PLACEHOLDER.search(text):
                if not allowed(rel, line):
                    bad.append((rel, i + 1, line.strip()))
    if bad:
        print("SQL text is built by pasting values into it. Use bound parameters (?1, ?2) instead:\n")
        for rel, n, text in bad:
            print(f"  {rel}:{n}: {text}")
        print("\nIf the pasted part is a fixed constant that can never come from outside,")
        print("add it to ALLOWED in scripts/check_sql_safety.py with the reason.")
        return 1
    print(f"OK: no SQL text built from outside values ({len(ALLOWED)} reviewed exceptions)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
