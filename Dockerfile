# Build with the repository's locked dependency graph and pinned Rust toolchain.
FROM rust:1.99-slim-bookworm AS build
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
COPY templates ./templates
COPY static ./static
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates sqlite3 curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 woodshop \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin woodshop \
    && install -d -o 10001 -g 10001 /data

COPY --from=build /build/target/release/woodshop /usr/local/bin/woodshop
ENV APP_ENV=production APP_PORT=8080 DATABASE_URL=sqlite:/data/woodshop.db DATA_DIR=/data
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/health >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/woodshop"]
