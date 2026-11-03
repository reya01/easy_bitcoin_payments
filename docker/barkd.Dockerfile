# syntax=docker/dockerfile:1
#
# barkd sidecar for Easy Bitcoin Payments, pinned to Second's PREBUILT
# barkd 0.8.0 release binary (GitLab release assets — the same binaries we
# live-verified on signet). Compiling Bark from source inside a small VPS
# build is slow and memory-hungry; the release binary avoids that.
#
# The downloaded binary is checksum-pinned below (sha256s recorded
# 2026-11-03 directly from the GitLab release assets). Before trusting this
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
      amd64) BARK_ARCH=x86_64; BARK_SHA=e78bb35dbfb4c9b639a505e12e8cf3c3ab0a730c0fa4d42032b053a7b9db2a14 ;; \
      arm64) BARK_ARCH=arm64; BARK_SHA=3575f6edf0c4091befd12eeff9d5fbb5958c3131e88c62611d6cfd8f9f5b0ebc ;; \
      *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac \
 && curl -fsSL "https://gitlab.com/api/v4/projects/ark-bitcoin%2Fbark/packages/generic/release-assets/bark-0.8.0/barkd-0.8.0-linux-${BARK_ARCH}" \
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
