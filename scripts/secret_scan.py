import os, re
pats = {
  'lntb (invoice)': r'lntbs?[0-9][a-zA-Z0-9]+',
  'bearer-ish token': r'(Bearer\s+[A-Za-z0-9_\-\.]{20,})',
  'email': r'[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-z]{2,}',
  'hex64 (txid/key)': r'\b[0-9a-f]{64}\b',
  'wif/key markers': r'(privkey|private_key|BEGIN\ (RSA\ )?PRIVATE|L1[a-zA-Z0-9]{51}|Kw[a-zA-Z0-9]{51}|5[HJK][1-9A-Za-z]{49})',
}
skip = {'.git', 'target', 'docs', 'umbrel'}
hits = []
for root, dirs, files in os.walk('.'):
    dirs[:] = [d for d in dirs if d not in skip]
    for f in files:
        p = os.path.join(root, f)
        try:
            t = open(p, encoding='utf-8', errors='ignore').read()
        except Exception:
            continue
        for name, pat in pats.items():
            if f == 'Cargo.lock' and name == 'hex64 (txid/key)':
                continue  # crate checksums, not secrets
            for m in re.finditer(pat, t):
                line = t[:m.start()].count('\n') + 1
                hits.append(f"{p}:{line}: {name}: {m.group(0)[:60]}")
print('\n'.join(hits) if hits else 'NO HITS')
