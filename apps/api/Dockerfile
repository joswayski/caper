FROM rust:1.94-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY apps/api/Cargo.toml apps/api/Cargo.toml
COPY apps/api/src apps/api/src
COPY apps/api/build.rs apps/api/build.rs
COPY apps/api/migrations apps/api/migrations

RUN cargo build --locked --release --package caper-api

FROM debian:bookworm-slim AS api

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 caper \
    && useradd --uid 10001 --gid caper --no-create-home --shell /usr/sbin/nologin caper

COPY --from=builder /build/target/release/caper-api /usr/local/bin/caper-api

USER 10001:10001
EXPOSE 3001
ENTRYPOINT ["/usr/local/bin/caper-api"]
