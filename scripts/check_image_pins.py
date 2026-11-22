#!/usr/bin/env python3
"""Every container image this project pulls must be pinned by digest.

A tag (debian:bookworm-slim, ubuntu:24.04) can be moved upstream to point at
different contents; a digest (@sha256:...) cannot. This checks:

  * every FROM line in Dockerfile and docker/*Dockerfile
    (references to an earlier build stage, and `scratch`, are fine);
  * every `image:` in docker-compose.yml (our own locally built image,
    written as ${EBP_IMAGE:-...}, is fine);
  * every public image named in a `docker run` / `docker pull` in the CI
    workflows.

Run from the repository root:  python3 scripts/check_image_pins.py
On failure it lists each unpinned image so it can be pinned.
"""
import glob
import re
import sys

bad = []  # (where, image)


def pinned(ref):
    return "@sha256:" in ref


# 1. Dockerfiles.
for path in ["Dockerfile"] + sorted(glob.glob("docker/*Dockerfile")):
    stages = set()
    for n, line in enumerate(open(path, encoding="utf-8"), 1):
        m = re.match(r"^\s*FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?", line, re.I)
        if not m:
            continue
        ref, alias = m.group(1), m.group(2)
        if ref.lower() not in stages and ref != "scratch" and not pinned(ref):
            bad.append((f"{path}:{n}", ref))
        if alias:
            stages.add(alias.lower())

# 2. Compose file.
for n, line in enumerate(open("docker-compose.yml", encoding="utf-8"), 1):
    m = re.match(r"^\s*image:\s*(\S+)", line)
    if m and not m.group(1).startswith("${") and not pinned(m.group(1)):
        bad.append((f"docker-compose.yml:{n}", m.group(1)))

# 3. Public images pulled in CI. Images we build ourselves in the same job
#    (easy-bitcoin-payments:smoke, ...) and shell variables are not pulls.
PUBLIC = re.compile(
    r"\b((?:docker\.io/|ghcr\.io/|quay\.io/)?"
    r"(?:library/)?(?:ubuntu|debian|alpine|busybox|caddy|rust|python|node|secondark/[\w.-]+)"
    r":[\w.-]+(?:@sha256:[0-9a-f]{64})?)"
)
for path in sorted(glob.glob(".github/workflows/*.yml")):
    for n, line in enumerate(open(path, encoding="utf-8"), 1):
        if not re.search(r"\bdocker\s+(run|pull|create)\b", line):
            continue
        for ref in PUBLIC.findall(line):
            if not pinned(ref):
                bad.append((f"{path}:{n}", ref))

if bad:
    print("Not pinned by digest (pin each as name:tag@sha256:<digest>):")
    for where, ref in bad:
        print(f"  {where}: {ref}")
    sys.exit(1)
print("All container images are pinned by digest.")
