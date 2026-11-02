# syntax=docker/dockerfile:1
#
# barkd sidecar for Easy Bitcoin Payments, pinned to a bark source tag.
#
# Second publishes no registry image we can digest-pin, so we build from the
# tag. Before trusting this image with mainnet funds, verify the tag's commit
# hash against Second's signed release notes and record it here. When a
# registry image appears, replace this file's use in docker-compose.yml
# with `image: <registry>/barkd@sha256:<digest>`.

ARG BARK_VERSION=0.8.0

FROM rust:1-bookworm AS builder
ARG BARK_VERSION
# VERIFY: tag format — Second tags bark releases as v<version>.
RUN git clone --depth 1 --branch "v${BARK_VERSION}" https://github.com/second-tech/bark.git /src
WORKDIR /src
RUN cargo build --release -p barkd

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10002 --create-home --home-dir /home/barkd barkd \
 && mkdir -p /data \
 && chown barkd:barkd /data
COPY --from=builder /src/target/release/barkd /usr/local/bin/barkd
USER barkd
VOLUME ["/data"]
EXPOSE 3000
ENTRYPOINT ["barkd"]