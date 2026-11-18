# syntax=docker/dockerfile:1
#
# barkd sidecar for Easy Bitcoin Payments, pinned to Second's PREBUILT
# barkd 0.8.1 release binary (GitLab release assets — the same binaries we
# live-verified on signet). Compiling Bark from source inside a small VPS
# build is slow and memory-hungry; the release binary avoids that.
#
# The downloaded binary is checksum-pinned below (sha256s recorded
# 2026-11-18, computed by CI and matching the SHA256SUMS on the GitLab release page). Before trusting this
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
      amd64) BARK_ARCH=x86_64; BARK_SHA=e634724a17796de352beb2e28808059dd3d0aef1beb3afd358ee3a582c20c85c ;; \
      arm64) BARK_ARCH=arm64; BARK_SHA=e781f4f85bbad52655d5f10369fd07fb3ae1a8b2a47f7e8f2b54fea42653e463 ;; \
      *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac \
 && curl -fsSL "https://gitlab.com/api/v4/projects/ark-bitcoin%2Fbark/packages/generic/release-assets/bark-0.8.1/barkd-0.8.1-linux-${BARK_ARCH}" \
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
