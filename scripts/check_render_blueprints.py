#!/usr/bin/env python3
"""Guards the Render blueprints against the mistakes that lose a merchant's
wallet or silently change their server: free plan, no disk, auto-deploy on,
or (for the image template) anything other than a pinned digest placeholder."""
import sys

import yaml


def load(path):
    with open(path) as f:
        return yaml.safe_load(f)


def check(path, image_variant):
    errs = []
    svc = (load(path).get("services") or [{}])[0]
    if svc.get("plan") in (None, "free"):
        errs.append("plan must be a paid plan (the free plan has no disk and erases the wallet)")
    if svc.get("autoDeploy") is not False:
        errs.append("autoDeploy must be false")
    disk = svc.get("disk") or {}
    if disk.get("mountPath") != "/data" or not disk.get("sizeGB"):
        errs.append("a persistent disk must be mounted at /data")
    if svc.get("healthCheckPath") != "/api/v1/health":
        errs.append("healthCheckPath must be /api/v1/health")
    if image_variant:
        if svc.get("runtime") != "image":
            errs.append("the image template must use runtime: image")
        url = (svc.get("image") or {}).get("url", "")
        if url != "IMAGE_DIGEST_FROM_RELEASE_NOTES" and "@sha256:" not in url:
            errs.append("image url must be the placeholder or an @sha256 digest, never a tag")
    else:
        if svc.get("runtime") != "docker":
            errs.append("render.yaml builds from the repository (runtime: docker)")
    return errs


bad = 0
for path, variant in (("render.yaml", False), ("deploy/render-image.yaml.template", True)):
    errs = check(path, variant)
    print(("FAIL " if errs else "PASS ") + path)
    for e in errs:
        print("   -", e)
    bad += bool(errs)
sys.exit(1 if bad else 0)
