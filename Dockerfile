# syntax=docker/dockerfile:1

# ---- build stage -----------------------------------------------------------
FROM rust:1.82-bookworm AS builder

WORKDIR /src

# Build dependencies first so source edits do not invalidate the dep cache.
# The lockfile is copied so image builds are reproducible and match CI.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && \
    printf 'fn main() {}\n' > src/main.rs && \
    printf '' > src/lib.rs && \
    cargo build --release && \
    rm -rf src

COPY src ./src
COPY migrations ./migrations
COPY assets ./assets

RUN cargo build --release

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