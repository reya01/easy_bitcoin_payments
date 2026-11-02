# syntax=docker/dockerfile:1
#
# barkd sidecar for Easy Bitcoin Payments, pinned to Second's PREBUILT
# barkd 0.8.0 release binary (GitLab release assets — the same binaries we
# live-verified on signet). Compiling Bark from source inside a small VPS
# build is slow and memory-hungry; the release binary avoids that.
#
# Before trusting this image with mainnet funds: verify the downloaded
# binary against the SHA256SUMS published with the GitLab release
# (https://gitlab.com/ark-bitcoin/bark/-/releases) and record the hash here.

ARG TARGETARCH

FROM debian:bookworm-slim
ARG TARGETARCH
# GitLab release-asset arch names differ from Docker's TARGETARCH values.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && case "$TARGETARCH" in \
      amd64) BARK_ARCH=x86_64 ;; \
      arm64) BARK_ARCH=arm64 ;; \
      *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac \
 && curl -fsSL "https://gitlab.com/api/v4/projects/ark-bitcoin%2Fbark/packages/generic/release-assets/bark-0.8.0/barkd-0.8.0-linux-${BARK_ARCH}" \
      -o /usr/local/bin/barkd \
 && chmod +x /usr/local/bin/barkd \
 && useradd --system --uid 10002 --create-home --home-dir /home/barkd barkd \
 && mkdir -p /data \
 && chown barkd:barkd /data
USER barkd
VOLUME ["/data"]
EXPOSE 3000
ENTRYPOINT ["barkd"]
