# syntax=docker/dockerfile:1
#
# barkd sidecar for Easy Bitcoin Payments, pinned to Second's PREBUILT
# barkd 0.9.0 release binary (GitLab release assets). 0.9.0 is checked by CI
# against our contract; it is still to be drilled on signet and mainnet before a
# release. Compiling Bark from source inside a small VPS
# build is slow and memory-hungry; the release binary avoids that.
#
# The downloaded binary is checksum-pinned below (sha256s computed by CI on
# 2026-11-20 from the GitLab release assets; they equal the release's own
# SHA256SUMS). Before trusting this
# image with mainnet funds: re-verify these hashes against the SHA256SUMS
# published on the GitLab release page
# (https://gitlab.com/ark-bitcoin/bark/-/releases) — the pins only guarantee
# the asset we downloaded once, not GitLab's own integrity.

FROM debian:bookworm-slim
ARG TARGETARCH
# GitLab release-asset arch names differ from Docker's TARGETARCH values.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && case "$TARGETARCH" in \
      amd64) BARK_ARCH=x86_64; BARK_SHA=840d559de5a6b744bac0fedeea66e32d985e01c62763393f1e896e0c53dc1eb4 ;; \
      arm64) BARK_ARCH=arm64; BARK_SHA=7dd4b94db8512cf021060c14828d916547326f93a625e78bfeb870ad3eadd344 ;; \
      *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac \
 && curl -fsSL "https://gitlab.com/api/v4/projects/ark-bitcoin%2Fbark/packages/generic/release-assets/bark-0.9.0/barkd-0.9.0-linux-${BARK_ARCH}" \
      -o /usr/local/bin/barkd \
 && echo "${BARK_SHA}  /usr/local/bin/barkd" | sha256sum -c - \
 && chmod +x /usr/local/bin/barkd \
 && useradd --system --uid 10002 --create-home --home-dir /home/barkd barkd \
 && mkdir -p /data \
 && chown barkd:barkd /data
USER barkd
VOLUME ["/data"]
EXPOSE 3000
ENTRYPOINT ["barkd"]
