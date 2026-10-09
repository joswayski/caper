FROM rust:1.94-bookworm AS builder

WORKDIR /build

# Compile the dependencies first, against stub sources, so they get a layer of
# their own: it only changes with Cargo.toml or Cargo.lock, and an API code change
# reuses it from the layer cache and recompiles just caper-api.
COPY Cargo.toml Cargo.lock ./
COPY apps/api/Cargo.toml apps/api/Cargo.toml
RUN mkdir apps/api/src \
    && echo 'fn main() {}' > apps/api/src/main.rs \
    && touch apps/api/src/lib.rs \
    && cargo build --locked --release --package caper-api \
    && rm -rf apps/api/src target/release/.fingerprint/caper-api-* target/release/deps/caper_api-* \
        target/release/deps/libcaper_api-* target/release/caper-api target/release/caper-api.d

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
