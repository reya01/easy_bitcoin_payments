import os, re
pats = {
  'bolt11 (invoice)': r'ln(bc|tb|tbs)[0-9][a-zA-Z0-9]+',
  'bearer-ish token': r'(Bearer\s+[A-Za-z0-9_\-\.]{20,})',
  'email': r'[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-z]{2,}',
  'hex64 (txid/key)': r'\b[0-9a-f]{64}\b',
  # WIF keys: 51-char payload + version/marker char = 52 total.
  # 5H/5J/5K (uncompressed), K/L (mainnet compressed), c/9 (testnet+signet).
  'wif/key markers': r'(privkey|private_key|BEGIN\ (RSA\ )?PRIVATE|\b[5KLc9][1-9A-HJ-NP-Za-km-z]{50,51}\b)',
  # BIP39 seed phrase heuristic: 12 lowercase space-separated dictionary-ish
  # words on one line. Over-matches prose rarely; a missed mnemonic is worse.
  'bip39-ish mnemonic': r'\b[a-z]{3,8}( [a-z]{3,8}){11}\b',
  'ebp barkd token': r'EBP_BARKD_TOKEN=\S{16,}',
}
skip = {'.git', 'target'}
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
import sys
strict = '--strict' in sys.argv[1:]
if hits:
    print('\n'.join(hits))
    sys.exit(1 if strict else 0)
print('NO HITS')
sys.exit(0)
