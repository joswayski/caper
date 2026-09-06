FROM rust:1.94-bookworm AS builder

WORKDIR /build

# Cargo must be able to load every workspace member even though only caper-api is built.
COPY Cargo.toml Cargo.lock ./
COPY apps/api/Cargo.toml apps/api/Cargo.toml
COPY apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/Cargo.toml
COPY apps/desktop/src-tauri/build.rs apps/desktop/src-tauri/build.rs
COPY apps/desktop/src-tauri/src apps/desktop/src-tauri/src
COPY apps/desktop/src-tauri/capabilities apps/desktop/src-tauri/capabilities
COPY apps/desktop/src-tauri/icons apps/desktop/src-tauri/icons
COPY apps/desktop/src-tauri/tauri.conf.json apps/desktop/src-tauri/tauri.conf.json
COPY apps/api/src apps/api/src

RUN cargo build --locked --release --package caper-api

FROM debian:bookworm-slim AS media

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 caper \
    && useradd --uid 10001 --gid caper --no-create-home --shell /usr/sbin/nologin caper

COPY --from=builder /build/target/release/caper-api /usr/local/bin/caper-api

USER 10001:10001
EXPOSE 3001
ENTRYPOINT ["/usr/local/bin/caper-api"]
