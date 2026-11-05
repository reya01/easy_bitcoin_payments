# syntax=docker/dockerfile:1

# ---- build stage -----------------------------------------------------------
FROM rust:1-bookworm AS builder

WORKDIR /src

# Build dependencies first so source edits do not invalidate the dep cache.
# The lockfile is copied so image builds are reproducible and match CI.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && \
    printf 'fn main() {}\n' > src/main.rs && \
    printf '' > src/lib.rs && \
    cargo build --release && \
    rm -f target/release/ebp && \
    rm -rf src
# rm -f above: the stub binary is deleted after warming the dep cache, so if
# the real build below were ever skipped, NO binary exists and the smoke test
# (and the COPY --from) fails the build loudly instead of shipping a stub.

COPY src ./src
COPY migrations ./migrations
COPY assets ./assets

# FORCE the real build: cargo skips relinking when source mtimes are older
# than the cached stub binary — git clone/pull gives sources old mtimes and
# Docker COPY preserves them, so the stub survived every VPS rebuild for 22h
# (proven locally 2026-11-03; docs/KNOWN_ISSUES.md). Refreshing mtimes forces
# the crate recompile; dependency artifacts stay cached.
RUN find src -type f -exec touch {} + && cargo build --release

# Smoke test: the dependency-cache step above builds a STUB binary (empty
# main) into target/release/ebp. If the real build ever silently doesn't
# replace it, the stub ships and the container exits 0 instantly with no
# logs — an undebuggable 22-hour restart loop (see docs/KNOWN_ISSUES.md).
# The real binary prints a USAGE block; the stub prints nothing.
RUN timeout 10 ./target/release/ebp --help 2>&1 | grep -qi usage

# ---- runtime stage ---------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --create-home --home-dir /home/ebp ebp \
 && mkdir -p /data \
 && chown ebp:ebp /data

COPY --from=builder /src/target/release/ebp /usr/local/bin/ebp

USER ebp
WORKDIR /home/ebp

ENV EBP_BIND_ADDR=0.0.0.0:8080 \
    EBP_DATABASE_PATH=/data/ebp.sqlite3 \
    EBP_LOG_LEVEL=info,tower_http=info

VOLUME ["/data"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/api/v1/health || exit 1

ENTRYPOINT ["ebp"]
CMD ["serve"]

# ---- managed stage (single-container install: EBP + barkd) ------------------
# This is the DEFAULT (last) stage, so a plain `docker build .` and one-click
# hosts get the single-container install. docker-compose.yml (two containers)
# builds `--target runtime` instead. EBP runs
# barkd itself (EBP_MANAGED_BARKD=true), so no token copying, .env editing or
# terminal commands are needed during setup. barkd is the SAME checksum-pinned
# release binary as docker/barkd.Dockerfile — keep the two pins identical.
FROM debian:bookworm-slim AS managed
ARG TARGETARCH
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
 && useradd --system --uid 10001 --create-home --home-dir /home/ebp ebp \
 && mkdir -p /data \
 && chown ebp:ebp /data
COPY --from=builder /src/target/release/ebp /usr/local/bin/ebp
USER ebp
WORKDIR /home/ebp
ENV EBP_BIND_ADDR=0.0.0.0:8080 \
    EBP_DATABASE_PATH=/data/ebp.sqlite3 \
    EBP_MANAGED_BARKD=true \
    EBP_LOG_LEVEL=info,tower_http=info
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/api/v1/health || exit 1
ENTRYPOINT ["ebp"]
CMD ["serve"]
